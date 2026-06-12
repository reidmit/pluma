// `$variant` payload helpers. A variant stores its payload *inline* (`p0`/`p1`
// for arity ≤ 2, or the whole array in `rest` for arity ≥ 3), so the hot
// construct/match paths — where the arity is statically known — read/write the
// inline slots directly. The two helpers here serve the generic consumers
// (eq/wire/to-string/…) and the cold/dynamic construction sites, which work with
// a payload of runtime-unknown arity and want the uniform `$valarray` shape.

use super::wat::Wat;
use crate::types;
use wasm_encoder::{Function, ValType};

/// `__variant_payload(value v) -> valarray` — the variant `v`'s payload as a
/// uniform array. Returns the `rest` array directly for arity ≥ 3; otherwise
/// materializes a small (0/1/2-element) array from the inline `p0`/`p1` slots.
pub(crate) fn build_variant_payload_fn() -> Function {
	let mut w = Wat::new(1);
	let v = w.param(0);
	let vr = w.local(types::variant_ref());
	let ar = w.local(ValType::I32);
	w.local_get(v).ref_cast(types::T_VARIANT).local_set(vr);
	w.local_get(vr)
		.struct_get(types::T_VARIANT, 3)
		.local_set(ar);
	// arity ≥ 3: the payload already lives in `rest` (non-null here).
	w.local_get(ar).i32(3).i32_ge_s().if_(|w| {
		w.local_get(vr)
			.struct_get(types::T_VARIANT, 6)
			.ref_cast(types::T_VALARRAY)
			.ret();
	});
	// arity 0: an empty array.
	w.local_get(ar).i32_eqz().if_(|w| {
		w.array_new_fixed(types::T_VALARRAY, 0).ret();
	});
	// arity 1: [p0].
	w.local_get(ar).i32(1).i32_eq().if_(|w| {
		w.local_get(vr)
			.struct_get(types::T_VARIANT, 4)
			.array_new_fixed(types::T_VALARRAY, 1)
			.ret();
	});
	// arity 2: [p0, p1].
	w.local_get(vr).struct_get(types::T_VARIANT, 4);
	w.local_get(vr).struct_get(types::T_VARIANT, 5);
	w.array_new_fixed(types::T_VALARRAY, 2);
	w.finish()
}

/// `__variant_from_array(i32 vtag, value name, valarray arr) -> value` — build a
/// `$variant` whose payload is `arr`, splitting it into the inline slots: the
/// first two elements into `p0`/`p1` for arity ≤ 2, else the whole array into
/// `rest` (so nothing is copied). The `name`/`vtag` come straight from the caller.
pub(crate) fn build_variant_from_array_fn() -> Function {
	let mut w = Wat::new(3);
	let vtag = w.param(0);
	let name = w.param(1);
	let arr = w.param(2);
	let n = w.local(ValType::I32);
	// `p0`/`p1`/`rest` default to null; fill them per the arity.
	let p0 = w.local(types::value_ref());
	let p1 = w.local(types::value_ref());
	let rest = w.local(types::valarray_ref_null());
	w.local_get(arr).array_len().local_set(n);
	w.local_get(n).i32(0).i32_gt_s().if_(|w| {
		w.local_get(arr)
			.i32(0)
			.array_get(types::T_VALARRAY)
			.local_set(p0);
	});
	w.local_get(n).i32(1).i32_gt_s().if_(|w| {
		w.local_get(arr)
			.i32(1)
			.array_get(types::T_VALARRAY)
			.local_set(p1);
	});
	// arity ≥ 3: stash the whole array in `rest` (p0/p1 stay null and are ignored).
	w.local_get(n).i32(2).i32_gt_s().if_(|w| {
		w.local_get(arr).local_set(rest);
	});
	w.i32(types::TAG_VARIANT)
		.local_get(vtag)
		.local_get(name)
		.local_get(n)
		.local_get(p0)
		.local_get(p1)
		.local_get(rest)
		.struct_new(types::T_VARIANT);
	w.finish()
}
