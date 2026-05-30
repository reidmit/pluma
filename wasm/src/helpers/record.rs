// Record field access + update (`__getfield`, `__record_update`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::types;

/// Build `__getfield(record, name) -> value`: linear-scan the record's
/// name-sorted `names` array, comparing each to `name` via `__eq`; return the
/// parallel `values` element on match. Traps if absent (the type checker
/// guarantees the field exists).
pub(crate) fn build_getfield_fn(eq_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (rec, name) = (w.param(0), w.param(1));
	let names = w.local(types::valarray_ref());
	let values = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

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
pub(crate) fn build_record_update_fn(eq_idx: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(3);
	let (rec, name, value) = (w.param(0), w.param(1), w.param(2));
	let names = w.local(types::valarray_ref());
	let values = w.local(types::valarray_ref());
	let new = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

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
	w.local_get(new)
		.i32(0)
		.local_get(values)
		.i32(0)
		.local_get(n)
		.array_copy(va, va);
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
