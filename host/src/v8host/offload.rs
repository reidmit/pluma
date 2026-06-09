// Native import callbacks for the shared offload subsystem (notes/IO.md): the reactor
// controls (`io-poll`/`io-unwatch`, driven by the in-wasm scheduler's block step + reap),
// shared by `std.sys.net` and every offload client, plus the v0 `offload-sleep` proving
// op. They shape an offload `OpResult` into the same `(status, n)` marshalling ABI the net
// ops use. The reactor itself (`crate::offload`) is engine-independent; these are the V8
// glue.

use std::time::Duration;

use super::Ctx;
use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem, read_str, set_pair};
use crate::offload::OpResult;

/// Shape a scalar offload `OpResult` (a count, or `nothing`) into `(status, n)`; an error
/// stashes its message in `last_error` (read back via `io-last-error`, like net/std.sys.io).
/// `status`: 0 ok, 2 error. `Bytes` is a byte op and never reaches here.
fn offload_scalar(ctx: &mut Ctx, res: OpResult) -> (i32, i32) {
	match res {
		OpResult::Nothing => (0, 0),
		OpResult::Count(n) => (0, n as i32),
		OpResult::Err(e) => {
			ctx.state.last_error = e;
			(2, 0)
		}
		OpResult::Bytes(_) => unreachable!("offload_scalar on a byte op"),
	}
}

/// `io-poll(i64 deadline) -> i32`: the reactor block step. Block until a parked socket is
/// ready *or* a worker completion lands (deadline as a JS BigInt; `-1` = indefinite),
/// returning one woken fid (`-1` on timeout / nothing pending).
pub(super) fn cb_io_poll(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let deadline = args
		.get(0)
		.to_big_int(scope)
		.map(|b| b.i64_value().0)
		.unwrap_or(-1);
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	rv.set_int32(ctx.state.reactor.poll(deadline));
}

/// `io-unwatch(i32 fid) -> ()`: drop a parked socket wait or an in-flight offload op on
/// cancellation / reaping (the reaped fiber's `wait::IO` registration). Idempotent.
pub(super) fn cb_io_unwatch(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	ctx.state.reactor.unwatch(fid);
}

/// `offload-sleep(i32 fid, i64 nanos) -> (i32 status, i32 n)`: the v0 proving op — sleep
/// `nanos` *on a worker thread* (not the scheduler), so the fiber parks while other fibers
/// run. Called twice like the net ops: the first call has no completed result, so it
/// submits the blocking sleep and reports would-block (status 1); after the wake re-runs
/// the parked task, the second call collects the now-ready `nothing` (status 0).
pub(super) fn cb_offload_sleep(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let nanos = args
		.get(1)
		.to_big_int(scope)
		.map(|b| b.i64_value().0)
		.unwrap_or(0);
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	match ctx.state.reactor.collect(fid) {
		Some(res) => {
			let (s, n) = offload_scalar(ctx, res);
			set_pair(scope, &mut rv, s, n);
		}
		None => {
			let dur = Duration::from_nanos(nanos.max(0) as u64);
			ctx.state.reactor.submit(
				fid,
				Box::new(move || {
					std::thread::sleep(dur);
					OpResult::Nothing
				}),
			);
			set_pair(scope, &mut rv, 1, 0); // would-block
		}
	}
}

// --- async fs (the BlockingPool's first real client, notes/IO.md v1) ---------------
//
// Each op is called twice like the net/offload ops. The first call reads the path/data
// out of scratch *on the scheduler thread* (only this thread may touch V8 memory), then
// submits an owned `std::fs` closure to the pool and reports would-block. The worker runs
// the blocking call off-thread; after the wake re-runs the parked task, the second call
// collects the result and marshals it back — bytes into the caller's `(dst, cap)` buffer
// (overflow → `read_stash` for `io-copyout`) exactly like the sync `fs.rs` reads.

/// `fs-read(i32 fid, i32 path_ptr, i32 path_len, i32 dst, i32 cap) -> (i32 status, i32
/// len)`: read a whole file as text on a worker thread. ok delivers the bytes into
/// `(dst, cap)` (true `len`, overflow stashed); `read_to_string` keeps it parity with the
/// sync `fs.read-file`. status: 0 ok, 1 would-block, 2 error.
pub(super) fn cb_fs_read(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let (pp, pl) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let (dst, cap) = (argi(scope, &args, 3), argi(scope, &args, 4));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	match ctx.state.reactor.collect(fid) {
		Some(OpResult::Bytes(b)) => {
			let n = deliver_read_v8(scope, mem, ctx, dst, cap, b);
			set_pair(scope, &mut rv, 0, n);
		}
		Some(OpResult::Err(e)) => {
			ctx.state.last_error = e;
			set_pair(scope, &mut rv, 2, 0);
		}
		Some(_) => unreachable!("fs-read collected a non-bytes result"),
		None => {
			let path = read_str(scope, mem, pp, pl);
			ctx.state.reactor.submit(
				fid,
				Box::new(move || match std::fs::read_to_string(&path) {
					Ok(s) => OpResult::Bytes(s.into_bytes()),
					Err(e) => OpResult::Err(e.to_string()),
				}),
			);
			set_pair(scope, &mut rv, 1, 0);
		}
	}
}

/// Shared body for `fs-write` / `fs-append` (`(fid, path_ptr, path_len, data_ptr,
/// data_len) -> (status, n)`): write `data` to `path` on a worker thread, ok = `nothing`.
fn fs_write_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	append: bool,
	rv: &mut v8::ReturnValue,
) {
	let fid = argi(scope, args, 0);
	let (pp, pl) = (argi(scope, args, 1), argi(scope, args, 2));
	let (dp, dl) = (argi(scope, args, 3), argi(scope, args, 4));
	let (ctx, mem) = ctx_and_mem(scope, args);
	match ctx.state.reactor.collect(fid) {
		Some(OpResult::Nothing) => set_pair(scope, rv, 0, 0),
		Some(OpResult::Err(e)) => {
			ctx.state.last_error = e;
			set_pair(scope, rv, 2, 0);
		}
		Some(_) => unreachable!("fs-write collected a non-nothing result"),
		None => {
			let path = read_str(scope, mem, pp, pl);
			let data = read_mem(scope, mem, dp.max(0) as usize, dl.max(0) as usize);
			ctx.state.reactor.submit(
				fid,
				Box::new(move || {
					let res = if append {
						use std::io::Write;
						std::fs::OpenOptions::new()
							.create(true)
							.append(true)
							.open(&path)
							.and_then(|mut f| f.write_all(&data))
					} else {
						std::fs::write(&path, &data)
					};
					match res {
						Ok(()) => OpResult::Nothing,
						Err(e) => OpResult::Err(e.to_string()),
					}
				}),
			);
			set_pair(scope, rv, 1, 0);
		}
	}
}

pub(super) fn cb_fs_write(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	fs_write_impl(s, &a, false, &mut r);
}

pub(super) fn cb_fs_append(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	fs_write_impl(s, &a, true, &mut r);
}
