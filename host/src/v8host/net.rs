// std.sys.net native import callbacks. Reuses the engine-independent `HostNet`/`NetRet`
// reactor (`crate::net`); these just shape a `NetRet` into the marshalling ABI. The
// multi-result ops return a `[status, n]` JS array (how V8 surfaces a multi-value wasm
// import result).

use super::Ctx;
use super::marshal::{argi, ctx_and_mem, read_mem, read_str, write_mem};
use crate::net::NetRet;

/// Set a multi-result return as a `[a, b]` JS array.
fn set_pair(scope: &mut v8::HandleScope, rv: &mut v8::ReturnValue, a: i32, b: i32) {
	let arr = v8::Array::new(scope, 2);
	let av: v8::Local<v8::Value> = v8::Integer::new(scope, a).into();
	arr.set_index(scope, 0, av);
	let bv: v8::Local<v8::Value> = v8::Integer::new(scope, b).into();
	arr.set_index(scope, 1, bv);
	rv.set(arr.into());
}

/// Shape a scalar `NetRet` (id / count / nothing) into `(status, n)`; an error stashes
/// its message in `last_error` (read back via `io-last-error`, like std.sys.io).
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

fn net_dial(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	connect: bool,
	rv: &mut v8::ReturnValue,
) {
	let (ap, al) = (argi(scope, args, 0), argi(scope, args, 1));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let addr = read_str(scope, mem, ap, al);
	let ret = if connect {
		ctx.state.net.connect(&addr)
	} else {
		ctx.state.net.listen(&addr)
	};
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, rv, s, n);
}
pub(super) fn cb_net_listen(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	net_dial(s, &a, false, &mut r);
}
pub(super) fn cb_net_connect(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	net_dial(s, &a, true, &mut r);
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
	let ret = ctx.state.net.try_accept(fid, lid);
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
	let ret = ctx.state.net.try_read(fid, cid, cap.max(0) as usize);
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
	let ret = ctx.state.net.try_write(fid, cid, &data);
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, &mut rv, s, n);
}

/// `net-poll(i64 deadline) -> i32`: block until a parked socket is ready (the deadline
/// arrives as a JS BigInt).
pub(super) fn cb_net_poll(
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
	rv.set_int32(ctx.state.net.poll(deadline));
}

pub(super) fn cb_net_unwatch(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	ctx.state.net.unwatch(fid);
}
