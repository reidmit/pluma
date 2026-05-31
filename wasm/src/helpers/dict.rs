// Dict helpers: insert/lookup/remove/map/filter over the insertion-ordered
// `$dict` entries array (linear scan via `__eq`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::{Local, Wat};
use crate::runtime::OptionLits;
use crate::types;

// ---------------------------------------------------------------------------
// Dict helpers. A `$dict` is `{tag, $valarray entries}` where each entry is a
// `$tuple (key, value)`. We linear-scan with `__eq` on keys — the VM's hash
// buckets are a pure accelerator, so insertion-order + structural key equality
// fully determine observable behavior. insert/lookup/remove DROP the hash
// method-dict the `where (hash k)` constraint passes (handled at the call site).
// ---------------------------------------------------------------------------

/// Emit the `$valarray` of the dict in `d`, i.e. `d.entries`.
fn dict_entries_of(w: &mut Wat, d: Local) {
	w.local_get(d)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 1);
}

/// Emit `entries[idx].elems[field]` — the key (field 0) or value (1) of the
/// `$tuple` entry at `idx` in the `$valarray` held in `arr`.
fn dict_entry_field(w: &mut Wat, arr: Local, idx: Local, field: i32) {
	w.local_get(arr).local_get(idx).array_get(types::T_VALARRAY);
	w.ref_cast(types::T_TUPLE).struct_get(types::T_TUPLE, 1);
	w.i32(field).array_get(types::T_VALARRAY);
}

/// Build `__dict_insert(dict, key, value) -> dict`: scan for `key` (via `__eq`);
/// replace its entry if present, else append. Returns a fresh `$dict`.
pub(crate) fn build_dict_insert_fn(eq_idx: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(3);
	let (d, k, v) = (w.param(0), w.param(1), w.param(2));
	let entries = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let found = w.local(ValType::I32);
	let new = w.local(types::valarray_ref());

	// `new[at] = tuple(k, v)`; `at` emits the destination index.
	let store_kv = |w: &mut Wat, at: &dyn Fn(&mut Wat)| {
		w.local_get(new);
		at(w);
		w.i32(types::TAG_TUPLE)
			.local_get(k)
			.local_get(v)
			.array_new_fixed(va, 2)
			.struct_new(types::T_TUPLE);
		w.array_set(va);
	};

	dict_entries_of(&mut w, d);
	w.local_set(entries);
	w.local_get(entries).array_len().local_set(n);
	w.i32(-1).local_set(found);
	w.i32(0).local_set(i);
	// Keys are unique, so the last (==only) match is the entry to replace.
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			dict_entry_field(w, entries, i, 0);
			w.local_get(k).call(eq_idx);
			w.if_(|w| {
				w.local_get(i).local_set(found);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// Pre-init `new` (a non-null local) so it is definitely-assigned on every path
	// — the validator does not merge assignments made only inside if/else arms.
	w.local_get(entries).local_set(new);
	w.local_get(found).i32(0).i32_ge_s();
	w.if_else(
		|w| {
			// Replace: new = copy of entries; new[found] = (k, v).
			w.local_get(n).array_new_default(va).local_set(new);
			w.copy_loop(va, new, None, entries, None, n);
			store_kv(w, &|w| {
				w.local_get(found);
			});
		},
		|w| {
			// Append: new = copy of entries grown by one; new[n] = (k, v).
			w.local_get(n)
				.i32(1)
				.i32_add()
				.array_new_default(va)
				.local_set(new);
			w.copy_loop(va, new, None, entries, None, n);
			store_kv(w, &|w| {
				w.local_get(n);
			});
		},
	);
	w.i32(types::TAG_DICT)
		.local_get(new)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_lookup(dict, key) -> option value`: linear scan via `__eq`.
pub(crate) fn build_dict_lookup_fn(eq_idx: u32, opt: OptionLits) -> Function {
	let mut w = Wat::new(2);
	let (d, k) = (w.param(0), w.param(1));
	let entries = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// Push a fresh `$str` for an interned data-segment literal.
	let str_lit = |w: &mut Wat, (off, len): (u32, u32)| {
		w.i32(types::TAG_STR);
		w.i32(off as i32);
		w.i32(len as i32);
		w.array_new_data(types::T_BYTES, 0);
		w.struct_new(types::T_STR);
	};

	dict_entries_of(&mut w, d);
	w.local_set(entries);
	w.local_get(entries).array_len().local_set(n);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			dict_entry_field(w, entries, i, 0);
			w.local_get(k).call(eq_idx);
			w.if_(|w| {
				// return some(value).
				w.i32(types::TAG_VARIANT).i32(opt.some_tag as i32);
				str_lit(w, opt.some_name);
				dict_entry_field(w, entries, i, 1);
				w.array_new_fixed(types::T_VALARRAY, 1)
					.struct_new(types::T_VARIANT)
					.ret();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// none.
	w.i32(types::TAG_VARIANT).i32(opt.none_tag as i32);
	str_lit(&mut w, opt.none_name);
	w.array_new_fixed(types::T_VALARRAY, 0)
		.struct_new(types::T_VARIANT);
	w.finish()
}

/// Build `__dict_remove(dict, key) -> dict`: drop the matching entry (renumbered
/// dense). Returns the original dict unchanged when the key is absent.
pub(crate) fn build_dict_remove_fn(eq_idx: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (d, k) = (w.param(0), w.param(1));
	let entries = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let found = w.local(ValType::I32);
	let new = w.local(types::valarray_ref());
	let src_off = w.local(ValType::I32);
	let seg_len = w.local(ValType::I32);

	dict_entries_of(&mut w, d);
	w.local_set(entries);
	w.local_get(entries).array_len().local_set(n);
	w.i32(-1).local_set(found);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			dict_entry_field(w, entries, i, 0);
			w.local_get(k).call(eq_idx);
			w.if_(|w| {
				w.local_get(i).local_set(found);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// Absent: hand back the original dict.
	w.local_get(found).i32(0).i32_lt_s();
	w.if_(|w| {
		w.local_get(d).ret();
	});
	// new = array(n-1); copy [0..found) then (found+1..n) shifted down by one.
	w.local_get(n)
		.i32(1)
		.i32_sub()
		.array_new_default(va)
		.local_set(new);
	// new[0..found) = entries[0..found).
	w.copy_loop(va, new, None, entries, None, found);
	// new[found..] = entries[found+1..n]; length = (n-1) - found.
	w.local_get(found).i32(1).i32_add().local_set(src_off);
	w.local_get(n)
		.i32(1)
		.i32_sub()
		.local_get(found)
		.i32_sub()
		.local_set(seg_len);
	w.copy_loop(va, new, Some(found), entries, Some(src_off), seg_len);
	w.i32(types::TAG_DICT)
		.local_get(new)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_map(dict, f) -> dict`: `f` over each value, keys preserved.
pub(crate) fn build_dict_map_fn(arity1: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (d, f) = (w.param(0), w.param(1));
	let entries = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let new = w.local(types::valarray_ref());
	let key = w.local(types::value_ref());
	let nv = w.local(types::value_ref());

	dict_entries_of(&mut w, d);
	w.local_set(entries);
	w.local_get(entries).array_len().local_set(n);
	w.local_get(n).array_new_default(va).local_set(new);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			dict_entry_field(w, entries, i, 0);
			w.local_set(key);
			// nv = f(value): env = f, arg = value, call_indirect.
			w.local_get(f).ref_cast(types::T_CLOSURE);
			dict_entry_field(w, entries, i, 1);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity1);
			w.local_set(nv);
			// new[i] = (key, nv).
			w.local_get(new).local_get(i);
			w.i32(types::TAG_TUPLE)
				.local_get(key)
				.local_get(nv)
				.array_new_fixed(va, 2)
				.struct_new(types::T_TUPLE);
			w.array_set(va);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_DICT)
		.local_get(new)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_filter(dict, f) -> dict`: keep entries where `f key value` is
/// true (the entry tuple is reused verbatim).
pub(crate) fn build_dict_filter_fn(arity2: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (d, f) = (w.param(0), w.param(1));
	let entries = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let tmp = w.local(types::valarray_ref());
	let write = w.local(ValType::I32);
	let key = w.local(types::value_ref());
	let v = w.local(types::value_ref());
	let out = w.local(types::valarray_ref());

	dict_entries_of(&mut w, d);
	w.local_set(entries);
	w.local_get(entries).array_len().local_set(n);
	w.local_get(n).array_new_default(va).local_set(tmp);
	w.i32(0).local_set(write);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			dict_entry_field(w, entries, i, 0);
			w.local_set(key);
			dict_entry_field(w, entries, i, 1);
			w.local_set(v);
			// keep = f(k, v): env = f, args k v, call_indirect; unbox the $bool.
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(key).local_get(v);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity2);
			w.ref_cast(types::T_BOOL).struct_get(types::T_BOOL, 1);
			w.if_(|w| {
				// tmp[write] = entry; write += 1.
				w.local_get(tmp).local_get(write);
				w.local_get(entries).local_get(i).array_get(va);
				w.array_set(va);
				w.local_get(write).i32(1).i32_add().local_set(write);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// out = array(write); copy tmp[0..write].
	w.local_get(write).array_new_default(va).local_set(out);
	w.copy_loop(va, out, None, tmp, None, write);
	w.i32(types::TAG_DICT)
		.local_get(out)
		.struct_new(types::T_DICT);
	w.finish()
}
