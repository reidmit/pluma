// std/sys/net native import callbacks. Reuses the engine-independent `HostNet`/`NetRet`
// reactor (`crate::net`); these just shape a `NetRet` into the marshalling ABI. The
// multi-result ops return a `[status, n]` JS array (how V8 surfaces a multi-value wasm
// import result).

use super::Ctx;
use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem, read_str, set_pair, write_mem};
use crate::net::NetRet;

/// Shape a scalar `NetRet` (id / count / nothing) into `(status, n)`; an error stashes
/// its message in `last_error` (read back via `io-last-error`, like std/sys/io).
fn net_scalar_v8(ctx: &mut Ctx, ret: NetRet) -> (i32, i32) {
	match ret {
		NetRet::OkInt(v) => (0, v),
		NetRet::OkNothing => (0, 0),
		NetRet::WouldBlock => (1, 0),
		NetRet::Err(e) => {
			ctx.state.last_error = e;
			(2, 0)
		}
		NetRet::OkBytes(_) | NetRet::OkStr(_) => unreachable!("net_scalar_v8 on a byte op"),
	}
}

/// Shape a byte-returning `NetRet` (read bytes / local-addr string) into `(status,
/// len)`, writing the payload into scratch at `dst` (truncated to `cap`).
fn net_bytes_v8(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ctx: &mut Ctx,
	dst: i32,
	cap: i32,
	ret: NetRet,
) -> (i32, i32) {
	let bytes = match ret {
		NetRet::OkBytes(b) => b,
		NetRet::OkStr(s) => s.into_bytes(),
		NetRet::WouldBlock => return (1, 0),
		NetRet::Err(e) => {
			ctx.state.last_error = e;
			return (2, 0);
		}
		NetRet::OkInt(_) | NetRet::OkNothing => unreachable!("net_bytes_v8 on a scalar op"),
	};
	let len = bytes.len().min(cap.max(0) as usize);
	write_mem(scope, mem, dst.max(0) as usize, &bytes[..len]);
	(0, len as i32)
}

pub(super) fn cb_net_listen(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (ap, al) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let addr = read_str(scope, mem, ap, al);
	let ret = ctx.state.net.listen(&addr);
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, &mut rv, s, n);
}

/// `net-connect(i32 fid, i32 addr_ptr, i32 addr_len) -> (i32 status, i32 conn-id)`: dial a
/// server, offloaded to a pool worker so the blocking DNS resolution + TCP handshake don't
/// stall the scheduler thread (host/src/offload.rs). Submit-or-collect like the other offload ops:
/// the first call submits the blocking `TcpStream::connect` and reports would-block (status
/// 1); after the wake re-runs the parked task, the second call adopts the connected socket
/// into the table and returns its id. status: 0 ok, 1 would-block, 2 error.
pub(super) fn cb_net_connect(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let (ap, al) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	match ctx.state.reactor.collect(fid) {
		Some(crate::offload::OpResult::Conn(stream)) => {
			let ret = ctx.state.net.adopt_conn(stream);
			let (s, n) = net_scalar_v8(ctx, ret);
			set_pair(scope, &mut rv, s, n);
		}
		Some(crate::offload::OpResult::Err(e)) => {
			ctx.state.last_error = e;
			set_pair(scope, &mut rv, 2, 0);
		}
		Some(_) => unreachable!("net-connect collected a non-conn result"),
		None => {
			let addr = read_str(scope, mem, ap, al);
			ctx.state.reactor.submit(
				fid,
				Box::new(move || match std::net::TcpStream::connect(&addr) {
					Ok(s) => crate::offload::OpResult::Conn(s),
					Err(e) => crate::offload::OpResult::Err(e.to_string()),
				}),
			);
			set_pair(scope, &mut rv, 1, 0);
		}
	}
}

pub(super) fn cb_net_close(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let id = argi(scope, &args, 0) as u32;
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.close(id);
	rv.set_int32(net_scalar_v8(ctx, ret).0);
}

pub(super) fn cb_net_local_addr(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let id = argi(scope, &args, 0) as u32;
	let (dst, cap) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.local_addr(id);
	let (s, n) = net_bytes_v8(scope, mem, ctx, dst, cap, ret);
	set_pair(scope, &mut rv, s, n);
}

pub(super) fn cb_net_accept(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let lid = argi(scope, &args, 1) as u32;
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.try_accept(&mut ctx.state.reactor, fid, lid);
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, &mut rv, s, n);
}

pub(super) fn cb_net_read(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let cid = argi(scope, &args, 1) as u32;
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let ret = ctx
		.state
		.net
		.try_read(&mut ctx.state.reactor, fid, cid, cap.max(0) as usize);
	let (s, n) = net_bytes_v8(scope, mem, ctx, dst, cap, ret);
	set_pair(scope, &mut rv, s, n);
}

pub(super) fn cb_net_write(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let cid = argi(scope, &args, 1) as u32;
	let (src, len) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let data = read_mem(scope, mem, src.max(0) as usize, len.max(0) as usize);
	let ret = ctx
		.state
		.net
		.try_write(&mut ctx.state.reactor, fid, cid, &data);
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, &mut rv, s, n);
}

/// `web-fetch(req_ptr, req_len, dst, cap) -> i32 len` — the `std/web/fetch` transport
/// (the browser's sync `fetch`, here a blocking HTTP/1.1 exchange over `std::net`).
/// Reads the request string, runs the exchange (`crate::net::web_fetch`), and delivers
/// the reply into the caller's `(dst, cap)` buffer (overflow → `read_stash` for
/// `io-copyout`), returning the true length; on failure stashes the message in
/// `last_error` and returns -1, which the wasm side shapes into `err` via `__io_result`.
pub(super) fn cb_web_fetch(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (rp, rl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let req = read_str(scope, mem, rp, rl);
	match crate::net::web_fetch(&req) {
		Ok(reply) => {
			let n = deliver_read_v8(scope, mem, ctx, dst, cap, reply.into_bytes());
			rv.set_int32(n);
		}
		Err(e) => {
			ctx.state.last_error = e;
			rv.set_int32(-1);
		}
	}
}
