// Unary float math — the libm calls (`(f64) -> f64`), one zero-sized fn item per
// function so each registers as a distinct `pluma.math-*` import.

/// A unary `(f64) -> f64` math import: apply `f` to arg 0.
fn math_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	rv: &mut v8::ReturnValue,
	f: fn(f64) -> f64,
) {
	let x = args.get(0).number_value(scope).unwrap_or(0.0);
	rv.set_double(f(x));
}
pub(super) fn cb_math_log(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::ln);
}
pub(super) fn cb_math_log10(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::log10);
}
pub(super) fn cb_math_log2(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::log2);
}
pub(super) fn cb_math_exp(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::exp);
}
pub(super) fn cb_math_sin(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::sin);
}
pub(super) fn cb_math_cos(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::cos);
}
