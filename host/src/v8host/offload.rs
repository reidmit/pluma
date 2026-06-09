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
		OpResult::Bytes(_) | OpResult::Conn(_) => unreachable!("offload_scalar on a byte/conn op"),
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

/// Deliver an fs `OpResult` into the caller's `(dst, cap)` read buffer + a `(status, len)`
/// pair, the shared tail of the async + sync fs callbacks. ok payload = the op's `Bytes`
/// (text, the dir blob, the stat record, a query's `"1"`/`"0"`), or empty for the void
/// ops (`Nothing`); `Err` stashes its message in `last_error`. status: 0 ok, 2 error.
fn deliver_fs(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ctx: &mut Ctx,
	dst: i32,
	cap: i32,
	res: OpResult,
) -> (i32, i32) {
	match res {
		OpResult::Bytes(b) => (0, deliver_read_v8(scope, mem, ctx, dst, cap, b)),
		OpResult::Nothing => (0, 0),
		OpResult::Err(e) => {
			ctx.state.last_error = e;
			(2, 0)
		}
		OpResult::Count(_) | OpResult::Conn(_) => unreachable!("fs op produced a Count/Conn"),
	}
}

/// `fs-op(i32 fid, i32 op, i32 path_ptr, i32 path_len, i32 data_ptr, i32 data_len, i32
/// dst, i32 cap) -> (i32 status, i32 len)`: run any `std.sys.fs` op (selected by `op`,
/// see `fsop::op`) on a worker thread. `data` is the write payload or the rename/copy
/// destination path; the ok payload comes back through `(dst, cap)` (overflow stashed for
/// `io-copyout`). Submit-or-collect like the net/sleep ops: the first call submits + would-
/// blocks (status 1), the wake's re-run collects the worker's result.
pub(super) fn cb_fs_op(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let op = argi(scope, &args, 1);
	let (pp, pl) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (dp, dl) = (argi(scope, &args, 4), argi(scope, &args, 5));
	let (dst, cap) = (argi(scope, &args, 6), argi(scope, &args, 7));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	match ctx.state.reactor.collect(fid) {
		Some(res) => {
			let (s, n) = deliver_fs(scope, mem, ctx, dst, cap, res);
			set_pair(scope, &mut rv, s, n);
		}
		None => {
			let path = read_str(scope, mem, pp, pl);
			let data = read_mem(scope, mem, dp.max(0) as usize, dl.max(0) as usize);
			ctx.state.reactor.submit(
				fid,
				Box::new(move || crate::fsop::dispatch(op, &path, &data)),
			);
			set_pair(scope, &mut rv, 1, 0);
		}
	}
}

/// `fs-op-sync(i32 op, i32 path_ptr, i32 path_len, i32 data_ptr, i32 data_len, i32 dst,
/// i32 cap) -> i32 len`: the synchronous `-sync` twin — run the op inline (blocking this
/// thread) and deliver into `(dst, cap)`, returning the byte length or `-1` on error (the
/// message stashed in `last_error`, shaped to `err` by the wasm side, like `io-read-file`).
pub(super) fn cb_fs_op_sync(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let op = argi(scope, &args, 0);
	let (pp, pl) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let (dp, dl) = (argi(scope, &args, 3), argi(scope, &args, 4));
	let (dst, cap) = (argi(scope, &args, 5), argi(scope, &args, 6));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let path = read_str(scope, mem, pp, pl);
	let data = read_mem(scope, mem, dp.max(0) as usize, dl.max(0) as usize);
	let (s, n) = deliver_fs(
		scope,
		mem,
		ctx,
		dst,
		cap,
		crate::fsop::dispatch(op, &path, &data),
	);
	rv.set_int32(if s == 0 { n } else { -1 });
}
