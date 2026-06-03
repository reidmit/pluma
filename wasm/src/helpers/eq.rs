// Structural equality (`__eq`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::types;

/// Build the structural-equality runtime helper `__eq(a, b) -> i32` (1 = equal).
/// Recursive over variants; loops over string bytes. Implements structural
/// `==`: same-typed operands (the type checker guarantees it), IEEE float compare
/// (so `nan != nan`), byte-exact strings. `self_idx` is `__eq`'s own wasm index
/// (for the variant-payload recursion). Tuples/lists/records are not yet handled
/// (they trap — a clear signal to implement them, not a silent wrong answer).
pub(crate) fn build_eq_fn(self_idx: u32, dict_eq_idx: u32) -> Function {
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

	// ta = tag(a); tb = tag(b); if ta != tb -> 0.
	w.local_get(a).value_tag().local_set(ta);
	w.local_get(b).value_tag().local_set(tb);
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
	// INT — unbox each operand (a small int rides as an `i31ref`, a large one as a
	// heap `$int`; the two forms must compare equal, so route both through
	// `unbox_int` rather than assuming a single box shape).
	w.local_get(ta).i32(types::TAG_INT).i32_eq();
	w.if_(|w| {
		w.local_get(a).unbox_int();
		w.local_get(b).unbox_int();
		w.i64_eq().ret();
	});
	// DURATION reuses the `$int` shape; compare its i64 nanos (`1s == 1000ms`).
	scalar(&mut w, types::TAG_DURATION, types::T_INT, |w| {
		w.i64_eq();
	});
	// INSTANT likewise reuses `$int`; compare its i64 unix-nanos.
	scalar(&mut w, types::TAG_INSTANT, types::T_INT, |w| {
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
	// `len_field` says where the logical length lives: `Some(f)` reads struct
	// field `f` (a `$list`'s length field, which can be < the array's capacity);
	// `None` uses `array.len` (tuples/records/variant payloads are exact-sized).
	let cmp_array = |w: &mut Wat, sty: u32, field: u32, len_field: Option<u32>| {
		w.local_get(a)
			.ref_cast(sty)
			.struct_get(sty, field)
			.local_set(pa);
		w.local_get(b)
			.ref_cast(sty)
			.struct_get(sty, field)
			.local_set(pb);
		// Lengths must match.
		match len_field {
			Some(lf) => {
				w.local_get(a)
					.ref_cast(sty)
					.struct_get(sty, lf)
					.local_set(n);
				w.local_get(b)
					.ref_cast(sty)
					.struct_get(sty, lf)
					.local_get(n)
					.i32_ne();
			}
			None => {
				w.local_get(pa).array_len().local_set(n);
				w.local_get(pb).array_len().local_get(n).i32_ne();
			}
		}
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
		cmp_array(w, types::T_VARIANT, 3, None);
	});
	// TUPLE / LIST: compare the element arrays. RECORD: compare the values arrays
	// (same type ⇒ same name-sorted fields, so positional value compare suffices).
	w.local_get(ta).i32(types::TAG_TUPLE).i32_eq();
	w.if_(|w| {
		cmp_array(w, types::T_TUPLE, 1, None);
	});
	w.local_get(ta).i32(types::TAG_LIST).i32_eq();
	w.if_(|w| {
		cmp_array(w, types::T_LIST, 1, Some(2));
	});
	w.local_get(ta).i32(types::TAG_RECORD).i32_eq();
	w.if_(|w| {
		cmp_array(w, types::T_RECORD, 2, None);
	});
	// REF: reference identity (`ref.eq`) — two
	// cells are equal iff they are the same cell, regardless of contents.
	w.local_get(ta).i32(types::TAG_REF).i32_eq();
	w.if_(|w| {
		w.local_get(a).local_get(b).ref_eq().ret();
	});
	// DICT: order-independent compare, delegated to `__dict_eq` — same size and
	// every entry of `a` present in `b` with an `__eq` value (insertion order and
	// internal table layout are not observable).
	w.local_get(ta).i32(types::TAG_DICT).i32_eq();
	w.if_(|w| {
		w.local_get(a).local_get(b).call(dict_eq_idx).ret();
	});
	// EXTERN: reference identity (`ref.eq` on the wrapper struct, like `$ref`) — a
	// host handle is equal only to itself. No Phase-1 value reaches this; it's the
	// `==` arm DOM/fetch handles (Phase 3) need.
	w.local_get(ta).i32(types::TAG_EXTERN).i32_eq();
	w.if_(|w| {
		w.local_get(a).local_get(b).ref_eq().ret();
	});
	// Unhandled (closure/ctor): not structurally compared.
	w.unreachable();
	w.finish()
}
