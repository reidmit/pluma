// std/random / std/uuid (the `Entropy` capability). The host generates these natively
// with the `rand`/`uuid` crates. i64 results cross as JS BigInt; byte/string results go
// through scratch (`deliver_read_v8`). Range/length validation lives in `std/random`
// (Pluma), so the raw `random-int-range`/`random-bytes` imports never fail.

use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_str};

pub(super) fn cb_random_int(
	scope: &mut v8::HandleScope,
	_a: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	use rand::RngExt as _;
	let n = rand::rng().random_range(0..i64::MAX);
	rv.set(v8::BigInt::new_from_i64(scope, n).into());
}

pub(super) fn cb_random_float(
	_s: &mut v8::HandleScope,
	_a: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	use rand::RngExt as _;
	rv.set_double(rand::rng().random::<f64>());
}

pub(super) fn cb_random_int_range(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	use rand::RngExt as _;
	let lo = args
		.get(0)
		.to_big_int(scope)
		.map(|b| b.i64_value().0)
		.unwrap_or(0);
	let hi = args
		.get(1)
		.to_big_int(scope)
		.map(|b| b.i64_value().0)
		.unwrap_or(0);
	// `std/random` guarantees `lo < hi` before calling.
	let n = rand::rng().random_range(lo..hi);
	rv.set(v8::BigInt::new_from_i64(scope, n).into());
}

pub(super) fn cb_random_bytes(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	use rand::Rng as _;
	let (n, dst, cap) = (
		argi(scope, &args, 0),
		argi(scope, &args, 1),
		argi(scope, &args, 2),
	);
	let (ctx, mem) = ctx_and_mem(scope, &args);
	// `std/random` guarantees `n >= 0` before calling.
	let mut buf = vec![0u8; n.max(0) as usize];
	rand::rng().fill_bytes(&mut buf);
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, buf));
}

pub(super) fn cb_uuid_v4(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let s = uuid::Uuid::new_v4().to_string();
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, s.into_bytes()));
}

pub(super) fn cb_uuid_v7(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let s = uuid::Uuid::now_v7().to_string();
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, s.into_bytes()));
}

pub(super) fn cb_uuid_parse(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let s = read_str(scope, mem, pp, pl);
	let n = match uuid::Uuid::try_parse(&s) {
		Ok(u) => deliver_read_v8(scope, mem, ctx, dst, cap, u.to_string().into_bytes()),
		Err(e) => {
			ctx.state.last_error = e.to_string();
			-1
		}
	};
	rv.set_int32(n);
}
