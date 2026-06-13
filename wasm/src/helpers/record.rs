// Record field access + update (`__getfield`, `__record_update`).

use crate::helpers::wat::Wat;
use crate::scan::StrPool;
use crate::types;
use ir::Repr;
use wasm_encoder::{Function, ValType};

/// Build `__denominalize(value) -> value`: if `value` is a *nominal* `$shapeN`
/// record (tag `TAG_SHAPE`), lift it to the uniform `$record` so the name-scanning
/// consumers (`__eq`/`__getfield`/`__tostring`/wire/`__hash`) can treat every record
/// uniformly; any other value (including an already-uniform `$record`) is returned
/// unchanged. The lift builds the uniform `$record` the same way a record literal
/// does: a name-sorted `names` array of `$str` constants plus a parallel `values`
/// array read out of the struct's inline fields, boxing each `F64` slot into a
/// `$float`. Dispatches on the runtime `shape_id` (read via the `$shape_hdr`
/// supertype) — *not* `ref.test`, since WasmGC canonicalizes two structurally-
/// identical shape structs (e.g. `{x,y}` and `{name,age}`, both
/// `{tag,shape_id,boxed,boxed}`) into one runtime type that `ref.test` can't tell
/// apart. Every shape is interned by the time this body is built (only IR-function
/// emission interns shapes), so the chain is complete. An empty program (no nominal
/// records) makes this the identity.
pub(crate) fn build_denominalize_fn(
	shapes: &[(u32, u32, ir::RecordShape)],
	strpool: &StrPool,
) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let v = w.param(0);
	let sid = w.local(ValType::I32);

	// Fast path: a non-nominal value flows straight through. Reading field 0 (the
	// tag) requires narrowing the `eqref` param to `$value` first.
	w.local_get(v)
		.ref_cast(types::T_VALUE)
		.struct_get(types::T_VALUE, 0)
		.i32(types::TAG_SHAPE)
		.i32_ne()
		.if_(|w| {
			w.local_get(v).ret();
		});

	// Nominal: read the `shape_id` via the `$shape_hdr` supertype, then rebuild the
	// uniform `$record` for the matching shape.
	w.local_get(v)
		.ref_cast(types::T_SHAPE_HDR)
		.struct_get(types::T_SHAPE_HDR, 1)
		.local_set(sid);
	for (type_idx, shape_id, shape) in shapes {
		let k = shape.fields.len() as u32;
		w.local_get(sid).i32(*shape_id as i32).i32_eq();
		w.if_(|w| {
			w.i32(types::TAG_RECORD);
			// names: one `$str` constant per field, in the shape's name-sorted order.
			for name in &shape.fields {
				let (off, len) = strpool
					.at
					.get(name)
					.copied()
					.expect("record field name in string pool");
				w.i32(types::TAG_STR)
					.i32(off as i32)
					.i32(len as i32)
					.array_new_data(types::T_BYTES, 0)
					.struct_new(types::T_STR);
			}
			w.array_new_fixed(va, k);
			// values: read each inline field, boxing an unboxed `F64` into a `$float`.
			for (i, repr) in shape.field_reprs.iter().enumerate() {
				let slot = (2 + i) as u32;
				if *repr == Repr::F64 {
					w.i32(types::TAG_FLOAT)
						.local_get(v)
						.ref_cast(*type_idx)
						.struct_get(*type_idx, slot)
						.struct_new(types::T_FLOAT);
				} else {
					w.local_get(v)
						.ref_cast(*type_idx)
						.struct_get(*type_idx, slot);
				}
			}
			w.array_new_fixed(va, k);
			w.struct_new(types::T_RECORD);
			w.ret();
		});
	}
	// The tag said `TAG_SHAPE`, so one shape_id must have matched.
	w.unreachable();
	w.finish()
}

/// Build `__getfield(record, name) -> value`: linear-scan the record's
/// name-sorted `names` array, comparing each to `name` via `__eq`; return the
/// parallel `values` element on match. Traps if absent (the type checker
/// guarantees the field exists).
pub(crate) fn build_getfield_fn(eq_idx: u32, denom_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (rec, name) = (w.param(0), w.param(1));
	let names = w.local(types::valarray_ref());
	let values = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// A nominal `$shapeN` reaching open field access (a field read on a record that
	// flowed through generic code) is lifted to the uniform `$record` first.
	w.local_get(rec).call(denom_idx).local_set(rec);
	w.local_get(rec)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 1)
		.local_set(names);
	w.local_get(rec)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 2)
		.local_set(values);
	w.local_get(names).array_len().local_set(n);
	w.i32(0).local_set(i);
	w.block("done", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("done"); // not found -> fall out (then trap)
			w.local_get(names).local_get(i).array_get(types::T_VALARRAY);
			w.local_get(name).call(eq_idx);
			w.if_(|w| {
				w.local_get(values)
					.local_get(i)
					.array_get(types::T_VALARRAY)
					.ret();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.unreachable();
	w.finish()
}

/// Build `__record_update(rec, name, value) -> rec`: a copy of `rec` with the
/// field named `name` overridden. Shares `rec`'s name array; copies its values
/// and replaces the matching slot (found via `__eq` on names).
pub(crate) fn build_record_update_fn(eq_idx: u32, denom_idx: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(3);
	let (rec, name, value) = (w.param(0), w.param(1), w.param(2));
	let names = w.local(types::valarray_ref());
	let values = w.local(types::valarray_ref());
	let new = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// Lift a nominal base to the uniform `$record` before the name-scanning copy.
	w.local_get(rec).call(denom_idx).local_set(rec);
	w.local_get(rec)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 1)
		.local_set(names);
	w.local_get(rec)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 2)
		.local_set(values);
	w.local_get(values).array_len().local_set(n);
	// new = copy of values.
	w.local_get(n).array_new_default(va).local_set(new);
	w.copy_loop(va, new, None, values, None, n);
	// find name; new[i] = value; stop.
	w.i32(0).local_set(i);
	w.block("done", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("done"); // not found -> done
			w.local_get(names).local_get(i).array_get(va);
			w.local_get(name).call(eq_idx);
			w.if_(|w| {
				w.local_get(new).local_get(i).local_get(value).array_set(va);
				w.br("done");
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_RECORD)
		.local_get(names)
		.local_get(new)
		.struct_new(types::T_RECORD);
	w.finish()
}

/// Build `__record_rest(rec, excluded) -> rec`: a uniform `$record` of `rec`'s
/// fields whose names are *not* in `excluded` (a `$list` of `$str`). Backs a
/// `...rest` binding on a uniform match subject — e.g. a nested inner record bound
/// from a field read. The rest length is `rec.len - excluded.len` (an open pattern
/// matches fields that are present, so every excluded name is in `rec`); for each
/// non-excluded slot it copies the (name, value) pair.
pub(crate) fn build_record_rest_fn(eq_idx: u32, denom_idx: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (rec, excluded) = (w.param(0), w.param(1));
	// Lift a nominal subject to the uniform `$record` before filtering its fields.
	w.local_get(rec).call(denom_idx).local_set(rec);
	let names = w.local(types::valarray_ref());
	let values = w.local(types::valarray_ref());
	let exnames = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let e = w.local(ValType::I32);
	let restlen = w.local(ValType::I32);
	let rn = w.local(types::valarray_ref());
	let rv = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let j = w.local(ValType::I32);
	let k = w.local(ValType::I32);
	let member = w.local(ValType::I32);
	let name_i = w.local(types::value_ref());

	w.local_get(rec)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 1)
		.local_set(names);
	w.local_get(rec)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 2)
		.local_set(values);
	// `excluded` is a `$list`; its elements (field 1) are the excluded `$str` names.
	w.local_get(excluded)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(exnames);
	w.local_get(names).array_len().local_set(n);
	w.local_get(exnames).array_len().local_set(e);
	w.local_get(n).local_get(e).i32_sub().local_set(restlen);
	w.local_get(restlen).array_new_default(va).local_set(rn);
	w.local_get(restlen).array_new_default(va).local_set(rv);
	w.i32(0).local_set(i);
	w.i32(0).local_set(j);
	w.block("done", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("done");
			w.local_get(names)
				.local_get(i)
				.array_get(va)
				.local_set(name_i);
			// member = exnames contains name_i?
			w.i32(0).local_set(member);
			w.i32(0).local_set(k);
			w.block("kdone", |w| {
				w.loop_("klp", |w| {
					w.local_get(k).local_get(e).i32_ge_s().br_if("kdone");
					w.local_get(exnames).local_get(k).array_get(va);
					w.local_get(name_i).call(eq_idx);
					w.if_(|w| {
						w.i32(1).local_set(member).br("kdone");
					});
					w.local_get(k).i32(1).i32_add().local_set(k);
					w.br("klp");
				});
			});
			// not a member -> copy (name, value) into the rest arrays at j.
			w.local_get(member).i32_eqz().if_(|w| {
				w.local_get(rn).local_get(j).local_get(name_i).array_set(va);
				w.local_get(rv)
					.local_get(j)
					.local_get(values)
					.local_get(i)
					.array_get(va)
					.array_set(va);
				w.local_get(j).i32(1).i32_add().local_set(j);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_RECORD)
		.local_get(rn)
		.local_get(rv)
		.struct_new(types::T_RECORD);
	w.finish()
}
