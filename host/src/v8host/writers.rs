// The stdout/stderr writers, the `io.fail` abort, and `float_to_str`. wasm already
// rendered any value to bytes (via `__tostring` or the raw `$bytes`), so these just
// shuttle scratch bytes to the io sink (or, for `float_to_str`, format a float into
// scratch).

use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem, write_mem};

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
	// While an `io.capture` block is live, divert both streams into its frame instead
	// of the real sinks, so a test can assert on exactly what a thunk printed.
	match ctx.state.capture.last_mut() {
		Some(frame) if to_err => frame.err.extend_from_slice(&bytes),
		Some(frame) => frame.out.extend_from_slice(&bytes),
		None if to_err => ctx.state.io.write_err(&bytes),
		None => ctx.state.io.write_out(&bytes),
	}
}

/// `io-capture-start(dst, cap) -> 0`: push a fresh capture frame (stdout + stderr).
/// Shares the `(dst, cap) -> len` read shape (it always "returns" the empty string,
/// len 0) so it rides the generic io-read marshalling; the Pluma wrapper ignores it.
pub(super) fn cb_capture_start(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	ctx.state.capture.push(crate::CaptureFrame::default());
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, Vec::new()));
}

/// `io-capture-out(dst, cap) -> len`: pop the top capture frame, deliver its stdout
/// through the `(dst, cap)` read path (overflow → `read_stash`), and park its stderr
/// for the immediately-following `io-capture-err`. An empty stack (unbalanced use)
/// yields the empty string.
pub(super) fn cb_capture_out(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let frame = ctx.state.capture.pop().unwrap_or_default();
	ctx.state.capture_err = frame.err;
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, frame.out));
}

/// `io-capture-err(dst, cap) -> len`: deliver the stderr parked by the preceding
/// `io-capture-end` (the second half of one `io.capture`). Drains it, so a stray call
/// yields the empty string.
pub(super) fn cb_capture_err(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = std::mem::take(&mut ctx.state.capture_err);
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, bytes));
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

/// SSR stubs for the two browser-only event accessors. `std/view` names them in
/// every handler closure, so a server build that *constructs* a view (to render
/// it with `view.to-string`) declares these imports even though `to-string` drops
/// the handlers and never calls them. Registered so the server module links; if
/// one were ever actually invoked under the sys host it returns the empty answer.
///
/// `event-target-value(externref, dst, cap) -> i32 len` — no value, length 0.
pub(super) fn cb_event_target_value(
	_scope: &mut v8::HandleScope,
	_args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	rv.set_int32(0);
}

/// `event-target-checked(externref, dst, cap) -> i32 len` — no value, length 0
/// (decodes to `""`, i.e. `false`, in `std/event`).
pub(super) fn cb_event_target_checked(
	_scope: &mut v8::HandleScope,
	_args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	rv.set_int32(0);
}

/// `event-prevent-default(externref) -> ()` — no-op.
pub(super) fn cb_event_prevent_default(
	_scope: &mut v8::HandleScope,
	_args: v8::FunctionCallbackArguments,
	_rv: v8::ReturnValue,
) {
}

/// `dom-child-at(externref, i32) -> externref` — SSR/hydration is browser-only, so
/// under the sys host this is never reached; registered only so an ungated
/// `pluma run`/`test` of `std/web/render` links. Returns undefined (a null node).
pub(super) fn cb_dom_child_at(
	_scope: &mut v8::HandleScope,
	_args: v8::FunctionCallbackArguments,
	_rv: v8::ReturnValue,
) {
}
