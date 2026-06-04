// The stdout/stderr writers, the `io.fail` abort, and `float_to_str`. wasm already
// rendered any value to bytes (via `__tostring` or the raw `$bytes`), so these just
// shuttle scratch bytes to the io sink (or, for `float_to_str`, format a float into
// scratch).

use super::marshal::{argi, ctx_and_mem, read_mem, write_mem};

/// The shared writer body: read the pre-rendered `(ptr, len)` bytes out of scratch and
/// write them to stdout/stderr, optionally newline-terminated. wasm already rendered
/// (via `__tostring` or the raw `$bytes`), so the host just shuttles bytes.
fn write_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	to_err: bool,
	newline: bool,
) {
	let (ptr, len) = (argi(scope, args, 0), argi(scope, args, 1));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let mut bytes = read_mem(scope, mem, ptr.max(0) as usize, len.max(0) as usize);
	if newline {
		bytes.push(b'\n');
	}
	if to_err {
		ctx.state.io.write_err(&bytes);
	} else {
		ctx.state.io.write_out(&bytes);
	}
}

pub(super) fn cb_print(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	write_impl(s, &a, false, true);
}
pub(super) fn cb_print_err(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	write_impl(s, &a, true, true);
}
pub(super) fn cb_write_out(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	write_impl(s, &a, false, false);
}
pub(super) fn cb_write_err(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	write_impl(s, &a, true, false);
}

/// `io-fail(ptr, len)`: stash the pre-rendered message host-side, then throw — the
/// `_entry` call unwinds, and the runner surfaces the stashed message as the
/// program's `runtime error: <msg>` status.
pub(super) fn cb_io_fail(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let (ptr, len) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = read_mem(scope, mem, ptr.max(0) as usize, len.max(0) as usize);
	ctx.state.fail = Some(String::from_utf8_lossy(&bytes).into_owned());
	let exc = v8::String::new(scope, "io.fail").unwrap();
	scope.throw_exception(exc.into());
}

/// `float_to_str(f64, ptr, cap) -> i32 len`: format the float in the canonical
/// to-string form, write its UTF-8 bytes into scratch at `ptr` (≤ cap), return the length.
pub(super) fn cb_float_to_str(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let n = args.get(0).number_value(scope).unwrap_or(0.0);
	let (ptr, cap) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let s = if n.fract() == 0.0 && n.is_finite() {
		format!("{n:.1}")
	} else {
		format!("{n}")
	};
	let bytes = s.into_bytes();
	let (_ctx, mem) = ctx_and_mem(scope, &args);
	if bytes.len() <= cap.max(0) as usize {
		write_mem(scope, mem, ptr.max(0) as usize, &bytes);
	}
	rv.set_int32(bytes.len() as i32);
}
