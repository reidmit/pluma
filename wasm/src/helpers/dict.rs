// `core.dict` as a **mutable** open-addressing hash table. `insert`/`remove` mutate
// the `$dict` in place (and return it, for call-site convenience); `dict` values
// therefore have reference semantics, like `ref` / `list.set`. (The previous
// representation was an immutable persistent hash-trie that path-copied O(log n)
// nodes per insert — correct and shareable, but every insert allocated; a mutable
// table allocates nothing in the steady state, which is what a tally loop wants.)
//
// Layout (see `types.rs`): a `$dict` is `{ tag, indices, order }`.
//   * `order` — a `$list` of `$dentry { key, value, hash }` in insertion order. Its
//     length is the live entry count: there are no tombstones, so iteration
//     (entries/keys/values/size) is a dense walk and `remove` rebuilds the table.
//   * `indices` — the open-addressing probe table, a power-of-two `$valarray`. Each
//     slot is null (empty) or an `i31`-boxed position into `order` (positions are
//     well under 2^30, so the box is an immediate — no allocation). Linear probing;
//     a slot collision compares the entry's cached `hash` (raw i64) before the
//     costlier `__eq`.
//
// Insert grows + rehashes `indices` at 0.75 load (the `order` list is untouched).
// `__hash`/`__eq` are shared with the rest of the runtime; entries cache `__hash`
// so resize and probing never recompute it.

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::{Local, Wat};
use crate::runtime::OptionLits;
use crate::types;

// FNV-1a 64-bit constants. `__hash` need only be *consistent with `__eq`* (equal
// values hash equal); the exact mixing is internal, so these match no other
// component — they're just a well-distributed standard hash.
const FNV_OFFSET: i64 = 0xcbf2_9ce4_8422_2325u64 as i64;
const FNV_PRIME: i64 = 0x0000_0100_0000_01b3;

// Initial probe-table capacity (a power of two); grows ×2 at 0.75 load.
const INITIAL_CAP: i32 = 8;

const VA: u32 = types::T_VALARRAY;

// $dict field indices.
const DICT_INDICES: u32 = 1;
const DICT_ORDER: u32 = 2;
// $dentry field indices.
const DENTRY_KEY: u32 = 1;
const DENTRY_VAL: u32 = 2;
const DENTRY_HASH: u32 = 3;
// $list field indices.
const LIST_ELEMS: u32 = 1;
const LIST_LEN: u32 = 2;

// ---------------------------------------------------------------------------
// Small emitters shared across the table helpers.
// ---------------------------------------------------------------------------

/// Push a fresh `$dentry { tag, key, value, hash }` (`hash` is a raw i64 local).
fn make_dentry(w: &mut Wat, key: Local, value: Local, hash: Local) {
	w.i32(0); // sentinel tag — a `$dentry` never reaches tag-inspecting code
	w.local_get(key);
	w.local_get(value);
	w.local_get(hash);
	w.struct_new(types::T_DENTRY);
}

/// Push a `$tuple(a, b)` (a 2-element `(key, value)` entry for `dict.entries`).
fn tuple2(w: &mut Wat, a: Local, b: Local) {
	w.i32(types::TAG_TUPLE);
	w.local_get(a);
	w.local_get(b);
	w.array_new_fixed(VA, 2);
	w.struct_new(types::T_TUPLE);
}

/// Push the `nothing` value (a typed null reference — no allocation).
fn push_nothing(w: &mut Wat) {
	w.ref_null(types::T_VALUE);
}

// ---------------------------------------------------------------------------
// `__hash` — a structural hash consistent with `__eq`.
// ---------------------------------------------------------------------------

/// Build `__hash(value) -> $int`. FNV-1a over the value's structure, mirroring
/// `__eq`'s shape so equal values hash equal: tag, then the scalar payload or the
/// recursively-hashed children. `self_idx` is `__hash`'s own index (child
/// recursion). `ref`/`dict` keys (and any unhandled tag) collapse to the
/// tag-only hash — correct (`__eq` still separates them in the bucket), just not
/// finely distributed; such keys are exotic.
pub(crate) fn build_hash_fn(self_idx: u32) -> Function {
	let mut w = Wat::new(1);
	let v = w.param(0);
	let ta = w.local(ValType::I32);
	let h = w.local(ValType::I64);
	let i = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let bytes = w.local(types::bytes_ref());
	let arr = w.local(types::valarray_ref());
	let f = w.local(ValType::F64);

	// `h = (h ^ x) * prime` for the i64 `x` on top of the stack.
	let mix = |w: &mut Wat, h: Local| {
		w.local_get(h)
			.i64_xor()
			.i64(FNV_PRIME)
			.i64_mul()
			.local_set(h);
	};

	// h = OFFSET; ta = tag(v); mix the tag so distinct types diverge.
	w.i64(FNV_OFFSET).local_set(h);
	w.local_get(v).value_tag().local_set(ta);
	w.local_get(ta).i64_extend_i32_u();
	mix(&mut w, h);

	// BOOL — mix the 0/1 payload.
	w.local_get(ta).i32(types::TAG_BOOL).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_BOOL)
			.struct_get(types::T_BOOL, 1);
		w.i64_extend_i32_u();
		mix(w, h);
	});
	// INT / DURATION / INSTANT — all the `$int` shape; mix the i64 payload.
	w.local_get(ta).i32(types::TAG_INT).i32_eq();
	w.local_get(ta).i32(types::TAG_DURATION).i32_eq();
	w.i32_or();
	w.local_get(ta).i32(types::TAG_INSTANT).i32_eq();
	w.i32_or();
	w.if_(|w| {
		// `unbox_int` handles a small-int `i31ref` and a heap `$int` alike; a
		// duration/instant is always a heap `$int` (its `ref.test i31` is false).
		w.local_get(v).unbox_int();
		mix(w, h);
	});
	// FLOAT — normalize ±0.0 to one bit pattern (they are `__eq`-equal), then mix
	// the bits. (NaN need not be self-equal: `__eq` says `nan != nan`, so two NaN
	// keys are distinct entries regardless of hash.)
	w.local_get(ta).i32(types::TAG_FLOAT).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_FLOAT)
			.struct_get(types::T_FLOAT, 1)
			.local_set(f);
		w.local_get(f).i64(0).f64_reinterpret_i64().f64_eq(); // f == 0.0 ?
		w.if_result(
			ValType::I64,
			|w| {
				w.i64(0);
			},
			|w| {
				w.local_get(f).i64_reinterpret_f64();
			},
		);
		mix(w, h);
	});
	// STR / BYTES (same `{tag, $bytes}` shape; the mixed tag already separates
	// them) — fold each byte.
	w.local_get(ta).i32(types::TAG_STR).i32_eq();
	w.local_get(ta).i32(types::TAG_BYTES).i32_eq();
	w.i32_or();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1)
			.local_set(bytes);
		w.local_get(bytes).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				w.local_get(bytes).local_get(i).array_get_u(types::T_BYTES);
				w.i64_extend_i32_u();
				mix(w, h);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
	});
	// VARIANT — mix the within-enum tag, then each payload element's hash (the
	// display name is ignored, matching `__eq`).
	w.local_get(ta).i32(types::TAG_VARIANT).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 1)
			.i64_extend_i32_u();
		mix(w, h);
		w.local_get(v)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 3)
			.local_set(arr);
		hash_elems(w, self_idx, arr, n, i, h, mix);
	});
	// TUPLE — mix each element's hash.
	w.local_get(ta).i32(types::TAG_TUPLE).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_TUPLE)
			.struct_get(types::T_TUPLE, 1)
			.local_set(arr);
		hash_elems(w, self_idx, arr, n, i, h, mix);
	});
	// LIST — mix each element's hash, over the logical length (field 2).
	w.local_get(ta).i32(types::TAG_LIST).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(arr);
		w.local_get(v)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 2)
			.local_set(n);
		hash_n(w, self_idx, arr, n, i, h, mix);
	});
	// RECORD — mix each value's hash (names ignored, matching `__eq`).
	w.local_get(ta).i32(types::TAG_RECORD).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_RECORD)
			.struct_get(types::T_RECORD, 2)
			.local_set(arr);
		hash_elems(w, self_idx, arr, n, i, h, mix);
	});
	// NOTHING / REF / DICT / CTOR / … — the tag-only hash already on `h`.

	// Box the accumulated hash.
	w.i32(types::TAG_INT).local_get(h).struct_new(types::T_INT);
	w.finish()
}

/// Emit: `n = array.len(arr); for i in 0..n { mix(h, __hash(arr[i])) }`.
fn hash_elems(
	w: &mut Wat,
	self_idx: u32,
	arr: Local,
	n: Local,
	i: Local,
	h: Local,
	mix: impl Fn(&mut Wat, Local),
) {
	w.local_get(arr).array_len().local_set(n);
	hash_n(w, self_idx, arr, n, i, h, mix);
}

/// Emit: `for i in 0..n { mix(h, __hash(arr[i])) }` (caller supplies `n`).
fn hash_n(
	w: &mut Wat,
	self_idx: u32,
	arr: Local,
	n: Local,
	i: Local,
	h: Local,
	mix: impl Fn(&mut Wat, Local),
) {
	w.i32(0).local_set(i);
	w.block("ebrk", |w| {
		w.loop_("elp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("ebrk");
			w.local_get(arr).local_get(i).array_get(VA);
			w.call(self_idx)
				.ref_cast(types::T_INT)
				.struct_get(types::T_INT, 1);
			mix(w, h);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("elp");
		});
	});
}

// ---------------------------------------------------------------------------
// Construction / size / iteration.
// ---------------------------------------------------------------------------

/// Build `__dict_empty(unit) -> $dict`: a fresh table — an `INITIAL_CAP`-slot empty
/// probe array and an empty `order` list. The arg (unit) is ignored.
pub(crate) fn build_dict_empty_fn() -> Function {
	let mut w = Wat::new(1);
	w.i32(types::TAG_DICT);
	w.i32(INITIAL_CAP).array_new_default(VA); // indices: all null
	// order = empty $list { tag, [], 0 }.
	w.i32(types::TAG_LIST);
	w.i32(0).array_new_default(VA);
	w.i32(0);
	w.struct_new(types::T_LIST);
	w.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_size(dict) -> $int`: the `order` list's length (the live count).
pub(crate) fn build_dict_size_fn() -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	w.i32(types::TAG_INT);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN);
	w.i64_extend_i32_u();
	w.struct_new(types::T_INT);
	w.finish()
}

/// Build `__dict_entries(dict) -> list (k, v)`: the `order` entries as
/// `$tuple(key, value)` in insertion order. Dense (no tombstones).
pub(crate) fn build_dict_entries_fn() -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	let order = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let out = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());

	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.local_set(order);
	w.local_get(order)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(order)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(len);
	w.local_get(len).array_new_default(VA).local_set(out);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("brk");
			w.local_get(elems)
				.local_get(i)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(entry);
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_set(k);
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_VAL)
				.local_set(v);
			w.local_get(out).local_get(i);
			tuple2(w, k, v);
			w.array_set(VA);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_LIST)
		.local_get(out)
		.local_get(len)
		.struct_new(types::T_LIST);
	w.finish()
}

// ---------------------------------------------------------------------------
// Probe: find, lookup.
// ---------------------------------------------------------------------------

/// Build `__dict_find(dict, key) -> $dentry|null`: linear-probe the table for
/// `key`, returning its entry (caller reads the value) or null when absent.
/// `hash_idx` = `__hash`, `eq_idx` = `__eq`.
pub(crate) fn build_dict_find_fn(hash_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let h = w.local(ValType::I64);
	let indices = w.local(types::valarray_ref());
	let elems = w.local(types::valarray_ref());
	let mask = w.local(ValType::I32);
	let slot = w.local(ValType::I32);
	let cur = w.local(types::value_ref());
	let pos = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());

	// h = unbox(__hash(key)).
	w.local_get(key)
		.call(hash_idx)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_set(h);
	// indices = dict.indices; elems = dict.order.elems.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES)
		.local_set(indices);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	// mask = |indices| - 1; slot = (i32) h & mask.
	w.local_get(indices)
		.array_len()
		.i32(1)
		.i32_sub()
		.local_set(mask);
	w.local_get(h)
		.i32_wrap_i64()
		.local_get(mask)
		.i32_and()
		.local_set(slot);
	w.loop_("probe", |w| {
		w.local_get(indices)
			.local_get(slot)
			.array_get(VA)
			.local_set(cur);
		// empty slot → absent.
		w.local_get(cur).ref_is_null();
		w.if_(|w| {
			push_nothing(w);
			w.ret();
		});
		// pos = unbox i31(cur); entry = order.elems[pos].
		w.local_get(cur).ref_cast_i31().i31_get_s().local_set(pos);
		w.local_get(elems)
			.local_get(pos)
			.array_get(VA)
			.ref_cast(types::T_DENTRY)
			.local_set(entry);
		// cached-hash match, then the real key compare.
		w.local_get(entry)
			.struct_get(types::T_DENTRY, DENTRY_HASH)
			.local_get(h)
			.i64_eq();
		w.if_(|w| {
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_get(key)
				.call(eq_idx);
			w.if_(|w| {
				w.local_get(entry).ret();
			});
		});
		// advance (linear probe, wrapping).
		w.local_get(slot)
			.i32(1)
			.i32_add()
			.local_get(mask)
			.i32_and()
			.local_set(slot);
		w.br("probe");
	});
	w.unreachable();
	w.finish()
}

/// Build `__dict_lookup(dict, key) -> option value`: `__dict_find` then wrap in
/// `some`/`none`. `find_idx` = `__dict_find`; `opt` builds the variant literals.
pub(crate) fn build_dict_lookup_fn(find_idx: u32, opt: OptionLits) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let e = w.local(types::value_ref());

	// Push a fresh `$str` for an interned data-segment literal (the variant name).
	let str_lit = |w: &mut Wat, (off, len): (u32, u32)| {
		w.i32(types::TAG_STR);
		w.i32(off as i32);
		w.i32(len as i32);
		w.array_new_data(types::T_BYTES, 0);
		w.struct_new(types::T_STR);
	};

	w.local_get(dict).local_get(key).call(find_idx).local_set(e);
	// null → none.
	w.local_get(e).ref_is_null();
	w.if_(|w| {
		w.i32(types::TAG_VARIANT).i32(opt.none_tag as i32);
		str_lit(w, opt.none_name);
		w.array_new_fixed(VA, 0).struct_new(types::T_VARIANT);
		w.ret();
	});
	// some(entry.value).
	w.i32(types::TAG_VARIANT).i32(opt.some_tag as i32);
	str_lit(&mut w, opt.some_name);
	w.local_get(e)
		.ref_cast(types::T_DENTRY)
		.struct_get(types::T_DENTRY, DENTRY_VAL);
	w.array_new_fixed(VA, 1).struct_new(types::T_VARIANT);
	w.finish()
}

// ---------------------------------------------------------------------------
// Mutation: grow, insert, remove.
// ---------------------------------------------------------------------------

/// Build `__dict_grow(dict) -> nothing`: double `indices` and rehash every `order`
/// entry into the new probe table (the `order` list is untouched). Uses each
/// entry's cached `hash`, so no `__hash` recompute.
pub(crate) fn build_dict_grow_fn() -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let newindices = w.local(types::valarray_ref());
	let mask = w.local(ValType::I32);
	let pos = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());
	let slot = w.local(ValType::I32);

	// elems = dict.order.elems; len = dict.order.length.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(len);
	// newindices = new VA(|dict.indices| * 2); mask = newcap - 1.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES)
		.array_len()
		.i32(1)
		.i32_shl()
		.local_set(mask); // reuse `mask` to hold newcap briefly
	w.local_get(mask)
		.array_new_default(VA)
		.local_set(newindices);
	w.local_get(mask).i32(1).i32_sub().local_set(mask);
	// for pos in 0..len: place i31(pos) at the first empty slot from hash.
	w.i32(0).local_set(pos);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(pos).local_get(len).i32_ge_s().br_if("brk");
			w.local_get(elems)
				.local_get(pos)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(entry);
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_HASH)
				.i32_wrap_i64()
				.local_get(mask)
				.i32_and()
				.local_set(slot);
			w.loop_("pr", |w| {
				w.local_get(newindices)
					.local_get(slot)
					.array_get(VA)
					.ref_is_null();
				w.if_else(
					|w| {
						// empty → place pos here.
						w.local_get(newindices).local_get(slot);
						w.local_get(pos).ref_i31();
						w.array_set(VA);
					},
					|w| {
						// occupied → advance and keep probing.
						w.local_get(slot)
							.i32(1)
							.i32_add()
							.local_get(mask)
							.i32_and()
							.local_set(slot);
						w.br("pr");
					},
				);
			});
			w.local_get(pos).i32(1).i32_add().local_set(pos);
			w.br("lp");
		});
	});
	// dict.indices = newindices.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.local_get(newindices)
		.struct_set(types::T_DICT, DICT_INDICES);
	push_nothing(&mut w);
	w.finish()
}

/// Build `__dict_insert(dict, key, val) -> nothing`: set `key`→`val` in place.
/// Grows at 0.75 load, then linear-probes: an existing key's value is overwritten
/// in place, else a new `$dentry` is appended to `order` and its position recorded
/// in the probe table. `hash_idx`/`eq_idx`/`grow_idx` = `__hash`/`__eq`/`__dict_grow`;
/// `push_idx` = `__list_push`.
pub(crate) fn build_dict_insert_fn(
	hash_idx: u32,
	eq_idx: u32,
	grow_idx: u32,
	push_idx: u32,
) -> Function {
	let mut w = Wat::new(3);
	let (dict, key, val) = (w.param(0), w.param(1), w.param(2));
	let h = w.local(ValType::I64);
	let order = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let indices = w.local(types::valarray_ref());
	let mask = w.local(ValType::I32);
	let slot = w.local(ValType::I32);
	let cur = w.local(types::value_ref());
	let pos = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());

	// h = unbox(__hash(key)).
	w.local_get(key)
		.call(hash_idx)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_set(h);
	// order = dict.order; len = order.length.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.local_set(order);
	w.local_get(order)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(len);
	// Grow if (len + 1) * 4 >= cap * 3 (i.e. load would exceed 0.75).
	w.local_get(len).i32(1).i32_add().i32(4).i32_mul();
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES)
		.array_len()
		.i32(3)
		.i32_mul();
	w.i32_ge_s();
	w.if_(|w| {
		w.local_get(dict).call(grow_idx).drop();
	});
	// Reload indices (grow may have swapped it) + order.elems.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES)
		.local_set(indices);
	w.local_get(indices)
		.array_len()
		.i32(1)
		.i32_sub()
		.local_set(mask);
	w.local_get(order)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	// slot = (i32) h & mask.
	w.local_get(h)
		.i32_wrap_i64()
		.local_get(mask)
		.i32_and()
		.local_set(slot);
	w.loop_("probe", |w| {
		w.local_get(indices)
			.local_get(slot)
			.array_get(VA)
			.local_set(cur);
		// empty slot → append a new entry at position `len`.
		w.local_get(cur).ref_is_null();
		w.if_(|w| {
			w.local_get(order);
			make_dentry(w, key, val, h);
			w.call(push_idx).drop(); // order.push(entry); position is `len`
			w.local_get(indices).local_get(slot);
			w.local_get(len).ref_i31();
			w.array_set(VA);
			push_nothing(w);
			w.ret();
		});
		// occupied → if it's our key, overwrite the value in place.
		w.local_get(cur).ref_cast_i31().i31_get_s().local_set(pos);
		w.local_get(elems)
			.local_get(pos)
			.array_get(VA)
			.ref_cast(types::T_DENTRY)
			.local_set(entry);
		w.local_get(entry)
			.struct_get(types::T_DENTRY, DENTRY_HASH)
			.local_get(h)
			.i64_eq();
		w.if_(|w| {
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_get(key)
				.call(eq_idx);
			w.if_(|w| {
				w.local_get(entry)
					.local_get(val)
					.struct_set(types::T_DENTRY, DENTRY_VAL);
				push_nothing(w);
				w.ret();
			});
		});
		// advance (linear probe, wrapping).
		w.local_get(slot)
			.i32(1)
			.i32_add()
			.local_get(mask)
			.i32_and()
			.local_set(slot);
		w.br("probe");
	});
	w.unreachable();
	w.finish()
}

/// Build `__dict_remove(dict, key) -> nothing`: drop `key` in place. Rebuilds the
/// table from the surviving entries (so `order` stays compact — there are no
/// tombstones) and swaps the new `indices`/`order` into the struct.
/// `empty_idx`/`insert_idx`/`eq_idx` = `__dict_empty`/`__dict_insert`/`__eq`.
pub(crate) fn build_dict_remove_fn(empty_idx: u32, insert_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let temp = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());

	// temp = empty().
	push_nothing(&mut w);
	w.call(empty_idx).local_set(temp);
	// elems/len of the old order.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(len);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("brk");
			w.local_get(elems)
				.local_get(i)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(entry);
			// keep entries whose key ≠ the removed key.
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_get(key)
				.call(eq_idx)
				.i32_eqz();
			w.if_(|w| {
				w.local_get(temp);
				w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_KEY);
				w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_VAL);
				w.call(insert_idx).drop();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// dict.indices = temp.indices; dict.order = temp.order.
	w.local_get(dict).ref_cast(types::T_DICT);
	w.local_get(temp)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES);
	w.struct_set(types::T_DICT, DICT_INDICES);
	w.local_get(dict).ref_cast(types::T_DICT);
	w.local_get(temp)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER);
	w.struct_set(types::T_DICT, DICT_ORDER);
	push_nothing(&mut w);
	w.finish()
}

// ---------------------------------------------------------------------------
// Equality, map, filter — all build a fresh dict (non-destructive).
// ---------------------------------------------------------------------------

/// Build `__dict_eq(a, b) -> i32`: equal iff same size and every entry of `a` is in
/// `b` with an `__eq` value (order-independent). `eq_idx` = `__eq`, `find_idx` =
/// `__dict_find`.
pub(crate) fn build_dict_eq_fn(eq_idx: u32, find_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let elems = w.local(types::valarray_ref());
	let la = w.local(ValType::I32);
	let lb = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());
	let eb = w.local(types::value_ref());

	// la / lb = order lengths; unequal size ⇒ unequal.
	w.local_get(a)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(la);
	w.local_get(b)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(lb);
	w.local_get(la).local_get(lb).i32_ne();
	w.if_(|w| {
		w.i32(0).ret();
	});
	w.local_get(a)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(la).i32_ge_s().br_if("brk");
			w.local_get(elems)
				.local_get(i)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(entry);
			// eb = find(b, a-key); absent ⇒ unequal.
			w.local_get(b);
			w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_KEY);
			w.call(find_idx).local_set(eb);
			w.local_get(eb).ref_is_null();
			w.if_(|w| {
				w.i32(0).ret();
			});
			// values must be `__eq`.
			w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_VAL);
			w.local_get(eb)
				.ref_cast(types::T_DENTRY)
				.struct_get(types::T_DENTRY, DENTRY_VAL);
			w.call(eq_idx).i32_eqz();
			w.if_(|w| {
				w.i32(0).ret();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(1);
	w.finish()
}

/// Build `__dict_map(dict, f) -> dict`: a fresh dict with `f` applied to each value
/// (keys + order preserved); `dict` is untouched. `empty_idx`/`insert_idx` =
/// `__dict_empty`/`__dict_insert`; `arity1` is `f`'s `(env, value)` indirect type.
pub(crate) fn build_dict_map_fn(empty_idx: u32, insert_idx: u32, arity1: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, f) = (w.param(0), w.param(1));
	let out = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());
	let newval = w.local(types::value_ref());

	push_nothing(&mut w);
	w.call(empty_idx).local_set(out);
	order_elems_len(&mut w, dict, elems, len);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("brk");
			w.local_get(elems)
				.local_get(i)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(entry);
			// newval = f(entry.value).
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_VAL);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity1);
			w.local_set(newval);
			// out.insert(key, newval).
			w.local_get(out);
			w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_KEY);
			w.local_get(newval);
			w.call(insert_idx).drop();
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(out);
	w.finish()
}

/// Build `__dict_filter(dict, f) -> dict`: a fresh dict of the entries where `f k v`
/// is true (order preserved); `dict` is untouched. `arity2` is `f`'s
/// `(env, key, value)` indirect type.
pub(crate) fn build_dict_filter_fn(empty_idx: u32, insert_idx: u32, arity2: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, f) = (w.param(0), w.param(1));
	let out = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());

	push_nothing(&mut w);
	w.call(empty_idx).local_set(out);
	order_elems_len(&mut w, dict, elems, len);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("brk");
			w.local_get(elems)
				.local_get(i)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(entry);
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_set(k);
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_VAL)
				.local_set(v);
			// keep = f(k, v).
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(k).local_get(v);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity2);
			w.ref_cast(types::T_BOOL).struct_get(types::T_BOOL, 1);
			w.if_(|w| {
				w.local_get(out)
					.local_get(k)
					.local_get(v)
					.call(insert_idx)
					.drop();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(out);
	w.finish()
}

/// Shared preamble for map/filter: `elems`/`len` = the backing array + length of
/// `dict.order` (the entries in insertion order).
fn order_elems_len(w: &mut Wat, dict: Local, elems: Local, len: Local) {
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(len);
}

/// Build `__dict_update(dict, key, f) -> nothing`: a single-probe read-modify-write.
/// Calls `f` with `some(current value)` (or `none` if absent) and stores its result
/// at `key` in place — overwriting an existing entry's value or appending a new one.
/// Structurally a fused `lookup`+`insert` (one hash, one probe). `f` should not
/// mutate this dict. `hash_idx`/`eq_idx`/`grow_idx`/`push_idx` =
/// `__hash`/`__eq`/`__dict_grow`/`__list_push`; `arity1` is `f`'s `(env, option v)`
/// indirect type; `opt` builds the `some`/`none` argument.
pub(crate) fn build_dict_update_fn(
	hash_idx: u32,
	eq_idx: u32,
	grow_idx: u32,
	push_idx: u32,
	arity1: u32,
	opt: OptionLits,
) -> Function {
	let mut w = Wat::new(3);
	let (dict, key, f) = (w.param(0), w.param(1), w.param(2));
	let h = w.local(ValType::I64);
	let order = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let indices = w.local(types::valarray_ref());
	let mask = w.local(ValType::I32);
	let slot = w.local(ValType::I32);
	let cur = w.local(types::value_ref());
	let pos = w.local(ValType::I32);
	let entry = w.local(types::dentry_ref());
	let newval = w.local(types::value_ref());

	let str_lit = |w: &mut Wat, (off, len): (u32, u32)| {
		w.i32(types::TAG_STR);
		w.i32(off as i32);
		w.i32(len as i32);
		w.array_new_data(types::T_BYTES, 0);
		w.struct_new(types::T_STR);
	};
	// Push `env` then call `f` (arity-1) on the option already on top of the stack.
	let call_f = |w: &mut Wat| {
		w.local_get(f)
			.ref_cast(types::T_CLOSURE)
			.struct_get(types::T_CLOSURE, 1);
		w.call_indirect(arity1);
	};

	// h = unbox(__hash(key)).
	w.local_get(key)
		.call(hash_idx)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_set(h);
	// order = dict.order; len = order.length.
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ORDER)
		.local_set(order);
	w.local_get(order)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(len);
	// Grow if (len + 1) * 4 >= cap * 3.
	w.local_get(len).i32(1).i32_add().i32(4).i32_mul();
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES)
		.array_len()
		.i32(3)
		.i32_mul();
	w.i32_ge_s();
	w.if_(|w| {
		w.local_get(dict).call(grow_idx).drop();
	});
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_INDICES)
		.local_set(indices);
	w.local_get(indices)
		.array_len()
		.i32(1)
		.i32_sub()
		.local_set(mask);
	w.local_get(order)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(h)
		.i32_wrap_i64()
		.local_get(mask)
		.i32_and()
		.local_set(slot);
	w.loop_("probe", |w| {
		w.local_get(indices)
			.local_get(slot)
			.array_get(VA)
			.local_set(cur);
		// empty slot → newval = f(none); append a new entry at position `len`.
		w.local_get(cur).ref_is_null();
		w.if_(|w| {
			w.local_get(f).ref_cast(types::T_CLOSURE); // env
			w.i32(types::TAG_VARIANT).i32(opt.none_tag as i32);
			str_lit(w, opt.none_name);
			w.array_new_fixed(VA, 0).struct_new(types::T_VARIANT);
			call_f(w);
			w.local_set(newval);
			w.local_get(order);
			make_dentry(w, key, newval, h);
			w.call(push_idx).drop();
			w.local_get(indices).local_get(slot);
			w.local_get(len).ref_i31();
			w.array_set(VA);
			push_nothing(w);
			w.ret();
		});
		// occupied → if it's our key, newval = f(some(value)); overwrite in place.
		w.local_get(cur).ref_cast_i31().i31_get_s().local_set(pos);
		w.local_get(elems)
			.local_get(pos)
			.array_get(VA)
			.ref_cast(types::T_DENTRY)
			.local_set(entry);
		w.local_get(entry)
			.struct_get(types::T_DENTRY, DENTRY_HASH)
			.local_get(h)
			.i64_eq();
		w.if_(|w| {
			w.local_get(entry)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_get(key)
				.call(eq_idx);
			w.if_(|w| {
				w.local_get(f).ref_cast(types::T_CLOSURE); // env
				w.i32(types::TAG_VARIANT).i32(opt.some_tag as i32);
				str_lit(w, opt.some_name);
				w.local_get(entry).struct_get(types::T_DENTRY, DENTRY_VAL);
				w.array_new_fixed(VA, 1).struct_new(types::T_VARIANT);
				call_f(w);
				w.local_set(newval);
				w.local_get(entry)
					.local_get(newval)
					.struct_set(types::T_DENTRY, DENTRY_VAL);
				push_nothing(w);
				w.ret();
			});
		});
		// advance (linear probe, wrapping).
		w.local_get(slot)
			.i32(1)
			.i32_add()
			.local_get(mask)
			.i32_and()
			.local_set(slot);
		w.br("probe");
	});
	w.unreachable();
	w.finish()
}

/// Build `__dict_clear(dict) -> nothing`: drop every entry, in place — reset to a
/// fresh empty probe table + empty `order` list.
pub(crate) fn build_dict_clear_fn() -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	// dict.indices = a fresh empty probe table.
	w.local_get(dict).ref_cast(types::T_DICT);
	w.i32(INITIAL_CAP).array_new_default(VA);
	w.struct_set(types::T_DICT, DICT_INDICES);
	// dict.order = a fresh empty $list.
	w.local_get(dict).ref_cast(types::T_DICT);
	w.i32(types::TAG_LIST);
	w.i32(0).array_new_default(VA);
	w.i32(0);
	w.struct_new(types::T_LIST);
	w.struct_set(types::T_DICT, DICT_ORDER);
	push_nothing(&mut w);
	w.finish()
}
