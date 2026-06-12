// `$tuple` element helpers. A tuple stores its elements *inline* — element `i` at
// field `2 + i` for `i < 3`, the rest in the `rest` overflow array — so the hot
// construct/index/destructure paths read/write the slots directly. The two helpers
// here serve the generic consumers (eq/wire/to-string) and the cold/dynamic
// construction sites, which work with elements as a uniform `$valarray`.

use super::wat::Wat;
use crate::types;
use wasm_encoder::{Function, ValType};

/// `__tuple_elems(value t) -> valarray` — the tuple `t`'s elements as a uniform
/// array. Builds one of length `arity` from the inline slots (`e0`/`e1`/`e2`) and
/// the `rest` overflow.
pub(crate) fn build_tuple_elems_fn() -> Function {
	let mut w = Wat::new(1);
	let v = w.param(0);
	let tr = w.local(types::tuple_ref());
	let n = w.local(ValType::I32);
	let out = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	w.local_get(v).ref_cast(types::T_TUPLE).local_set(tr);
	w.local_get(tr).struct_get(types::T_TUPLE, 1).local_set(n);
	// out = new valarray(n).
	w.local_get(n)
		.array_new_default(types::T_VALARRAY)
		.local_set(out);
	// Copy the inline slots present (min(n, 3)).
	let copy_slot = |w: &mut Wat, slot: u32| {
		w.local_get(n).i32(slot as i32).i32_gt_s().if_(|w| {
			w.local_get(out).i32(slot as i32);
			w.local_get(tr).struct_get(types::T_TUPLE, 2 + slot);
			w.array_set(types::T_VALARRAY);
		});
	};
	copy_slot(&mut w, 0);
	copy_slot(&mut w, 1);
	copy_slot(&mut w, 2);
	// Copy any overflow (arity > 3): out[3..n] = rest[0..n-3].
	w.local_get(n).i32(3).i32_gt_s().if_(|w| {
		w.i32(3).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				w.local_get(out).local_get(i);
				w.local_get(tr)
					.struct_get(types::T_TUPLE, 5)
					.ref_cast(types::T_VALARRAY);
				w.local_get(i).i32(3).i32_sub().array_get(types::T_VALARRAY);
				w.array_set(types::T_VALARRAY);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
	});
	w.local_get(out);
	w.finish()
}

/// `__tuple_from_array(valarray arr) -> value` — build a `$tuple` whose elements are
/// `arr`, splitting it into the inline slots. The first three go in `e0`/`e1`/`e2`;
/// any beyond are copied into the `rest` overflow array (arity ≤ 3 leaves it null).
pub(crate) fn build_tuple_from_array_fn() -> Function {
	let mut w = Wat::new(1);
	let arr = w.param(0);
	let n = w.local(ValType::I32);
	let e0 = w.local(types::value_ref());
	let e1 = w.local(types::value_ref());
	let e2 = w.local(types::value_ref());
	let rest = w.local(types::valarray_ref_null());
	let i = w.local(ValType::I32);
	w.local_get(arr).array_len().local_set(n);
	w.local_get(n).i32(0).i32_gt_s().if_(|w| {
		w.local_get(arr)
			.i32(0)
			.array_get(types::T_VALARRAY)
			.local_set(e0);
	});
	w.local_get(n).i32(1).i32_gt_s().if_(|w| {
		w.local_get(arr)
			.i32(1)
			.array_get(types::T_VALARRAY)
			.local_set(e1);
	});
	w.local_get(n).i32(2).i32_gt_s().if_(|w| {
		w.local_get(arr)
			.i32(2)
			.array_get(types::T_VALARRAY)
			.local_set(e2);
	});
	// arity > 3: rest = copy of arr[3..n].
	w.local_get(n).i32(3).i32_gt_s().if_(|w| {
		w.local_get(n)
			.i32(3)
			.i32_sub()
			.array_new_default(types::T_VALARRAY)
			.local_set(rest);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i)
					.local_get(n)
					.i32(3)
					.i32_sub()
					.i32_ge_s()
					.br_if("brk");
				w.local_get(rest).local_get(i);
				w.local_get(arr)
					.local_get(i)
					.i32(3)
					.i32_add()
					.array_get(types::T_VALARRAY);
				w.array_set(types::T_VALARRAY);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
	});
	w.i32(types::TAG_TUPLE)
		.local_get(n)
		.local_get(e0)
		.local_get(e1)
		.local_get(e2)
		.local_get(rest)
		.struct_new(types::T_TUPLE);
	w.finish()
}
