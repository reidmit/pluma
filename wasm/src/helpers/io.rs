// The `core.io` result shaper. `core.io` host imports traffic only in primitive
// `$value`s (string / bytes / list-of-string / nothing) plus a null-on-failure
// signal — they never build the `result` enum. `__io_result` does that wrapping in
// wasm, so a real server/browser host (Rust/WASI/JS) never needs the `result`
// variant tags or layout. See `host_sig`/the emit call site for the contract.

use wasm_encoder::Function;

use crate::helpers::wat::Wat;
use crate::runtime::IoResultLits;
use crate::types;

/// Push a `$str` value for an interned data-segment string `(off, len)`.
fn str_lit(w: &mut Wat, (off, len): (u32, u32)) {
	w.i32(types::TAG_STR);
	w.i32(off as i32);
	w.i32(len as i32);
	w.array_new_data(types::T_BYTES, 0);
	w.struct_new(types::T_STR);
}

/// `__io_result(payload) -> result`. The argument is a `core.io` host import's
/// return: a primitive `$value` on success, or `null` on failure (the host having
/// stashed the message for `io-last-error`). Build `ok payload` (non-null) or
/// `err (io-last-error())` (null). `io_last_error` is that host import's index.
pub(crate) fn build_io_result_fn(io_last_error: u32, lits: IoResultLits) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let payload = w.param(0);

	w.local_get(payload).ref_is_null();
	w.if_result(
		types::value_ref(),
		// null host return -> `err (io-last-error())`.
		|w| {
			w.i32(types::TAG_VARIANT).i32(lits.err_tag as i32);
			str_lit(w, lits.err_name);
			w.call(io_last_error)
				.array_new_fixed(va, 1)
				.struct_new(types::T_VARIANT);
		},
		// non-null host return -> `ok payload`.
		|w| {
			w.i32(types::TAG_VARIANT).i32(lits.ok_tag as i32);
			str_lit(w, lits.ok_name);
			w.local_get(payload)
				.array_new_fixed(va, 1)
				.struct_new(types::T_VARIANT);
		},
	);
	w.finish()
}
