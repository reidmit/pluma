// Structural equality (`__eq`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::types;

/// Build the structural-equality runtime helper `__eq(a, b) -> i32` (1 = equal).
/// Recursive over variants; loops over string bytes. Mirrors `vm`'s structural
/// `==`: same-typed operands (the type checker guarantees it), IEEE float compare
/// (so `nan != nan`), byte-exact strings. `self_idx` is `__eq`'s own wasm index
/// (for the variant-payload recursion). Tuples/lists/records are not yet handled
/// (they trap — a clear signal to implement them, not a silent wrong answer).
pub(crate) fn build_eq_fn(self_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let ta = w.local(ValType::I32);
	let tb = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let aa = w.local(types::bytes_ref());
	let bb = w.local(types::bytes_ref());
	let pa = w.local(types::valarray_ref());
	let pb = w.local(types::valarray_ref());
	let j = w.local(ValType::I32);
	let found = w.local(ValType::I32);

	// ta = tag(a); tb = tag(b); if ta != tb -> 0.
	w.local_get(a).struct_get(types::T_VALUE, 0).local_set(ta);
	w.local_get(b).struct_get(types::T_VALUE, 0).local_set(tb);
	w.local_get(ta).local_get(tb).i32_ne();
	w.if_(|w| {
		w.i32(0).ret();
	});

	// Per-tag scalar case: compare the boxed payload at field 1 via `eqop`, return.
	let scalar = |w: &mut Wat, tag: i32, ty: u32, eqop: fn(&mut Wat)| {
		w.local_get(ta).i32(tag).i32_eq();
		w.if_(|w| {
			w.local_get(a).ref_cast(ty).struct_get(ty, 1);
			w.local_get(b).ref_cast(ty).struct_get(ty, 1);
			eqop(w);
			w.ret();
		});
	};
	// NOTHING -> equal.
	w.local_get(ta).i32(types::TAG_NOTHING).i32_eq();
	w.if_(|w| {
		w.i32(1).ret();
	});
	scalar(&mut w, types::TAG_BOOL, types::T_BOOL, |w| {
		w.i32_eq();
	});
	scalar(&mut w, types::TAG_INT, types::T_INT, |w| {
		w.i64_eq();
	});
	// DURATION reuses the `$int` shape; compare its i64 nanos (`1s == 1000ms`).
	scalar(&mut w, types::TAG_DURATION, types::T_INT, |w| {
		w.i64_eq();
	});
	scalar(&mut w, types::TAG_FLOAT, types::T_FLOAT, |w| {
		w.f64_eq();
	});

	// STR / BYTES (same `{tag, $bytes}` shape): equal lengths and equal bytes.
	w.local_get(ta).i32(types::TAG_STR).i32_eq();
	w.local_get(ta).i32(types::TAG_BYTES).i32_eq();
	w.i32_or();
	w.if_(|w| {
		w.local_get(a)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1)
			.local_set(aa);
		w.local_get(b)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1)
			.local_set(bb);
		w.local_get(aa).array_len().local_set(n);
		w.local_get(bb).array_len().local_get(n).i32_ne();
		w.if_(|w| {
			w.i32(0).ret();
		});
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				w.local_get(aa).local_get(i).array_get_u(types::T_BYTES);
				w.local_get(bb).local_get(i).array_get_u(types::T_BYTES);
				w.i32_ne();
				w.if_(|w| {
					w.i32(0).ret();
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.i32(1).ret();
	});

	// Element-wise array compare (recursive): load the `$valarray` at field `field`
	// of both operands (cast to `sty`), check equal lengths, then compare each
	// element via `__eq`; emit the success `return 1`.
	let cmp_array = |w: &mut Wat, sty: u32, field: u32| {
		w.local_get(a)
			.ref_cast(sty)
			.struct_get(sty, field)
			.local_set(pa);
		w.local_get(b)
			.ref_cast(sty)
			.struct_get(sty, field)
			.local_set(pb);
		// Lengths must match.
		w.local_get(pa).array_len().local_set(n);
		w.local_get(pb).array_len().local_get(n).i32_ne();
		w.if_(|w| {
			w.i32(0).ret();
		});
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				w.local_get(pa).local_get(i).array_get(types::T_VALARRAY);
				w.local_get(pb).local_get(i).array_get(types::T_VALARRAY);
				w.call(self_idx).i32_eqz();
				w.if_(|w| {
					w.i32(0).ret();
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.i32(1).ret();
	};
	// VARIANT: equal tags, then equal payloads.
	w.local_get(ta).i32(types::TAG_VARIANT).i32_eq();
	w.if_(|w| {
		w.local_get(a)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 1);
		w.local_get(b)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 1);
		w.i32_ne();
		w.if_(|w| {
			w.i32(0).ret();
		});
		cmp_array(w, types::T_VARIANT, 3);
	});
	// TUPLE / LIST: compare the element arrays. RECORD: compare the values arrays
	// (same type ⇒ same name-sorted fields, so positional value compare suffices).
	w.local_get(ta).i32(types::TAG_TUPLE).i32_eq();
	w.if_(|w| {
		cmp_array(w, types::T_TUPLE, 1);
	});
	w.local_get(ta).i32(types::TAG_LIST).i32_eq();
	w.if_(|w| {
		cmp_array(w, types::T_LIST, 1);
	});
	w.local_get(ta).i32(types::TAG_RECORD).i32_eq();
	w.if_(|w| {
		cmp_array(w, types::T_RECORD, 2);
	});
	// REF: reference identity (`ref.eq`), matching the VM's `Rc::ptr_eq` — two
	// cells are equal iff they are the same cell, regardless of contents.
	w.local_get(ta).i32(types::TAG_REF).i32_eq();
	w.if_(|w| {
		w.local_get(a).local_get(b).ref_eq().ret();
	});
	// DICT: order-independent structural compare (matches the VM). Equal sizes,
	// then every entry of `a` must have a key in `b` with an equal value. Keys are
	// unique within each dict, so equal sizes make this a bijection check. Entry
	// fields are read inline: `entries[idx]` is a `$tuple`, elem 0 = key, 1 = value.
	let entry_field = |w: &mut Wat, arr, idx, field: i32| {
		w.local_get(arr).local_get(idx).array_get(types::T_VALARRAY);
		w.ref_cast(types::T_TUPLE).struct_get(types::T_TUPLE, 1);
		w.i32(field).array_get(types::T_VALARRAY);
	};
	w.local_get(ta).i32(types::TAG_DICT).i32_eq();
	w.if_(|w| {
		// pa = a.entries; pb = b.entries; n = len(a); bail if lengths differ.
		w.local_get(a)
			.ref_cast(types::T_DICT)
			.struct_get(types::T_DICT, 1)
			.local_set(pa);
		w.local_get(b)
			.ref_cast(types::T_DICT)
			.struct_get(types::T_DICT, 1)
			.local_set(pb);
		w.local_get(pa).array_len().local_set(n);
		w.local_get(pb).array_len().local_get(n).i32_ne();
		w.if_(|w| {
			w.i32(0).ret();
		});
		w.i32(0).local_set(i);
		w.block("outer", |w| {
			w.loop_("oloop", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("outer"); // done; all matched
				w.i32(0).local_set(j);
				w.i32(0).local_set(found);
				w.block("inner", |w| {
					w.loop_("iloop", |w| {
						w.local_get(j).local_get(n).i32_ge_s().br_if("inner"); // key absent in b
						// if __eq(a.key[i], b.key[j]) { values must match }
						entry_field(w, pa, i, 0);
						entry_field(w, pb, j, 0);
						w.call(self_idx);
						w.if_(|w| {
							entry_field(w, pa, i, 1);
							entry_field(w, pb, j, 1);
							w.call(self_idx).i32_eqz();
							w.if_(|w| {
								w.i32(0).ret();
							});
							w.i32(1).local_set(found);
							w.br("inner"); // key found, move to next a-entry
						});
						w.local_get(j).i32(1).i32_add().local_set(j);
						w.br("iloop");
					});
				});
				// a-key absent in b -> not equal.
				w.local_get(found).i32_eqz();
				w.if_(|w| {
					w.i32(0).ret();
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("oloop");
			});
		});
		w.i32(1).ret();
	});
	// Unhandled (closure/ctor): not structurally compared.
	w.unreachable();
	w.finish()
}
