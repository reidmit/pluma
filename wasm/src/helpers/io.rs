// The `std/sys/io` result shaper. `std/sys/io` host imports traffic only in primitive
// `$value`s (string / bytes / list-of-string / nothing) plus a null-on-failure
// signal — they never build the `result` enum. `__io_result` does that wrapping in
// wasm, so a real server/browser host (Rust/WASI/JS) never needs the `result`
// variant tags or layout. See `host_sig`/the emit call site for the contract.

use crate::helpers::wat::Wat;
use crate::runtime::IoResultLits;
use crate::types;
use wasm_encoder::{Function, ValType};

/// Cap for the `io-last-error` message read. Errno strings are short; the host
/// truncates to this and returns the written length, so no overflow path is needed.
const ERR_CAP: i32 = 256;

/// `__io_result(payload) -> result`. The argument is a marshalled `std/sys/io` op's
/// shaped return: a primitive `$value` on success, or `null` on failure (the host
/// having stashed the message for `io-last-error`). Build `ok payload` (non-null) or
/// `err (io-last-error())` (null). The `err` message is read out of scratch:
/// `io_last_error(dst, cap) -> len` writes the message there; `__load_bytes` copies
/// it into a `$str`. `alloc`/`load_bytes`/`bump` are the marshalling helper/global.
pub(crate) fn build_io_result_fn(
	io_last_error: u32,
	alloc: u32,
	load_bytes: u32,
	bump: u32,
	lits: IoResultLits,
) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let payload = w.param(0);
	let dst = w.local(ValType::I32);
	let len = w.local(ValType::I32);

	w.local_get(payload).ref_is_null();
	w.if_result(
		types::value_ref(),
		// null host return -> `err (io-last-error())`.
		|w| {
			w.i32(types::TAG_VARIANT).i32(lits.err_tag as i32);
			w.i32(lits.err_gid as i32); // ctor_id (field 2)
			w.i32(1); // arity
			// p0 = message = $str(__load_bytes(dst, io_last_error(dst, ERR_CAP))).
			w.i32(0).global_set(bump);
			w.i32(ERR_CAP).call(alloc).local_set(dst);
			w.local_get(dst)
				.i32(ERR_CAP)
				.call(io_last_error)
				.local_set(len);
			w.i32(types::TAG_STR);
			w.local_get(dst).local_get(len).call(load_bytes);
			w.struct_new(types::T_STR);
			// p1, rest null, then the variant.
			w.ref_null(types::T_VALUE)
				.ref_null(va)
				.struct_new(types::T_VARIANT);
		},
		// non-null host return -> `ok payload`.
		|w| {
			w.i32(types::TAG_VARIANT).i32(lits.ok_tag as i32);
			w.i32(lits.ok_gid as i32); // ctor_id (field 2)
			w.i32(1); // arity
			w.local_get(payload) // p0
				.ref_null(types::T_VALUE)
				.ref_null(va)
				.struct_new(types::T_VARIANT);
		},
	);
	w.finish()
}
