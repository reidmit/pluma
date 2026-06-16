// `std/dict` as an **immutable persistent** hash-array-mapped trie. `insert`/
// `remove`/`update` path-copy the nodes on the root→leaf path and return a *new*
// `$dict`; every untouched subtree is shared by reference, so `dict` has value
// semantics like a record (and two consecutive versions can be diffed cheaply —
// shared subtrees are `ref.eq`).
//
// Layout (see `types.rs`):
//   * `$dict` is `{ tag, root, size }` — `root` is the top `$cnode` (or null when
//     empty), `size` caches the live entry count for O(1) `dict.size`.
//   * `$cnode` is `{ tag(=TAG_CNODE), dataMap, nodeMap, entries, children, edit }`.
//     This is the **HAMT** layout: an *uncompressed* `WIDTH`-wide
//     node — `entries` is a 32-slot `$valarray` indexed directly by the 5-bit hash
//     chunk; a slot is null (empty), a `$dentry` leaf, or a child `$cnode`, told
//     apart by reading field 0 (`TAG_CNODE` vs `$dentry`'s sentinel). `dataMap` is
//     a `BUCKET` sentinel marking a flat collision bucket (distinct keys whose full
//     64-bit hash collides — astronomically rare, kept only for correctness). The
//     CHAMP compaction (two bitmaps + `popcnt`-indexed compact arrays) is the
//     follow-on tuning step; `nodeMap`/`children`/`edit` are reserved for it.
//   * `$dentry` is `{ tag, key, value, i64 hash }` — `hash` caches `__hash(key)`.
//
// The recursive trie ops (`__cnode_*`) take the full key `hash` and current bit
// `shift` as *boxed* ints (computed once at the public wrapper, threaded down — the
// hash box is reused, shifts are `i31` immediates), so they're ordinary `Ty::Helper`
// functions recursing through their own index.

use crate::helpers::wat::{Local, Wat};
use crate::runtime::OptionLits;
use crate::types;
use wasm_encoder::{Function, ValType};

// FNV-1a 64-bit constants. `__hash` need only be *consistent with `__eq`* (equal
// values hash equal); the exact mixing is internal, so these match no other
// component — they're just a well-distributed standard hash.
const FNV_OFFSET: i64 = 0xcbf2_9ce4_8422_2325u64 as i64;
const FNV_PRIME: i64 = 0x0000_0100_0000_01b3;

const VA: u32 = types::T_VALARRAY;

/// Build a `none` `$variant` inline (arity 0, all payload slots null).
pub(crate) fn build_none(w: &mut Wat, opt: OptionLits) {
	w.i32(types::TAG_VARIANT).i32(opt.none_tag as i32);
	w.i32(opt.none_gid as i32); // ctor_id (field 2)
	w.i32(0)
		.ref_null(types::T_VALUE)
		.ref_null(types::T_VALUE)
		.ref_null(VA)
		.struct_new(types::T_VARIANT);
}

/// Push a `some` `$variant`'s header (tag, vtag, ctor_id, arity 1); the caller then
/// pushes the single payload value (`p0`) and calls `finish_some`.
pub(crate) fn start_some(w: &mut Wat, opt: OptionLits) {
	w.i32(types::TAG_VARIANT).i32(opt.some_tag as i32);
	w.i32(opt.some_gid as i32); // ctor_id (field 2)
	w.i32(1);
}

/// Close a `some` build started by `start_some`, with `p0` already on the stack:
/// null `p1`/`rest`, then the struct.
pub(crate) fn finish_some(w: &mut Wat) {
	w.ref_null(types::T_VALUE)
		.ref_null(VA)
		.struct_new(types::T_VARIANT);
}

// $dict field indices.
const DICT_ROOT: u32 = 1;
const DICT_SIZE: u32 = 2;
// $cnode field indices. A node is `{ tag, dataMap, nodeMap, entries, children, edit }`:
// `entries` is the compact `$dentry` array (one per set `dataMap` bit, indexed by
// `popcnt(dataMap & (bit-1))`), `children` the compact child-`$cnode` array (one per
// set `nodeMap` bit). A collision bucket is a node with both maps 0 and a flat
// `entries` of same-hash leaves.
const CN_DATAMAP: u32 = 1;
const CN_NODEMAP: u32 = 2;
const CN_ENTRIES: u32 = 3;
const CN_CHILDREN: u32 = 4;
const CN_EDIT: u32 = 5;
// $dentry field indices.
const DENTRY_KEY: u32 = 1;
const DENTRY_VAL: u32 = 2;
const DENTRY_HASH: u32 = 3;
// $list field indices.
const LIST_ELEMS: u32 = 1;
const LIST_LEN: u32 = 2;
// $tuple field index.
// $closure field index (the indirect fn slot).
const CLOSURE_FN: u32 = 1;

// ---------------------------------------------------------------------------
// Small emitters shared across the trie helpers.
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
	w.i32(2); // arity
	w.local_get(a); // e0
	w.local_get(b); // e1
	w.ref_null(types::T_VALUE); // e2
	w.ref_null(VA); // rest
	w.struct_new(types::T_TUPLE);
}

/// Push the `nothing` value (a typed null reference — no allocation).
fn push_nothing(w: &mut Wat) {
	w.ref_null(types::T_VALUE);
}

/// Push an empty (0-length) `$valarray`.
fn push_empty(w: &mut Wat) {
	w.i32(0).array_new_default(VA);
}

/// Build a `$cnode { tag, dataMap, nodeMap, entries, children, null edit }` from the
/// given bitmaps + compact arrays (all locals).
fn build_cnode(w: &mut Wat, data_map: Local, node_map: Local, entries: Local, children: Local) {
	w.i32(types::TAG_CNODE);
	w.local_get(data_map);
	w.local_get(node_map);
	w.local_get(entries);
	w.local_get(children);
	push_nothing(w); // edit (null = frozen)
	w.struct_new(types::T_CNODE);
}

/// Push a collision-bucket `$cnode` — both maps 0, a flat `entries` of same-hash
/// `$dentry` leaves, empty children.
fn make_bucket(w: &mut Wat, entries: Local) {
	w.i32(types::TAG_CNODE);
	w.i32(0); // dataMap
	w.i32(0); // nodeMap
	w.local_get(entries);
	push_empty(w); // children
	push_nothing(w); // edit
	w.struct_new(types::T_CNODE);
}

/// Build a `$cnode` like [`build_cnode`] but stamped with the transient owner
/// `token` (instead of a null edit), so a later transient insert in the same
/// session recognizes it as owned and mutates it in place.
fn build_cnode_t(
	w: &mut Wat,
	data_map: Local,
	node_map: Local,
	entries: Local,
	children: Local,
	token: Local,
) {
	w.i32(types::TAG_CNODE);
	w.local_get(data_map);
	w.local_get(node_map);
	w.local_get(entries);
	w.local_get(children);
	w.local_get(token);
	w.struct_new(types::T_CNODE);
}

/// Push a fresh transient owner token — a bare `$value` used only for `ref.eq`
/// identity (never tag-inspected).
fn push_token(w: &mut Wat) {
	w.i32(0).struct_new(types::T_VALUE);
}

/// Copy a `$valarray` of length `len` into a fresh one; return the new array.
fn copy_all(w: &mut Wat, src: Local, len: Local) -> Local {
	let new = w.local(types::valarray_ref());
	w.local_get(len).array_new_default(VA).local_set(new);
	w.copy_loop(VA, new, None, src, None, len);
	new
}

/// Push the 5-bit hash chunk at the current `shift`: `(hash >>> shift) & 31`.
fn push_chunk(w: &mut Wat, hash64: Local, shift64: Local) {
	w.local_get(hash64)
		.local_get(shift64)
		.i64_shr_u()
		.i32_wrap_i64()
		.i32(31)
		.i32_and();
}

/// Push the compact-array index of `bit`'s slot: `popcnt(map & (bit - 1))`.
fn push_idx(w: &mut Wat, map: Local, bit: Local) {
	w.local_get(map)
		.local_get(bit)
		.i32(1)
		.i32_sub()
		.i32_and()
		.i32_popcnt();
}

/// Copy `src` (length `len`) with slot `idx` replaced by `elem`; return the new array.
fn copy_replace(w: &mut Wat, src: Local, len: Local, idx: Local, elem: Local) -> Local {
	let new = w.local(types::valarray_ref());
	w.local_get(len).array_new_default(VA).local_set(new);
	w.copy_loop(VA, new, None, src, None, len);
	w.local_get(new)
		.local_get(idx)
		.local_get(elem)
		.array_set(VA);
	new
}

/// Copy `src` (length `len`) with `elem` spliced in at `idx`; return the new array
/// (length `len + 1`).
fn splice_insert(w: &mut Wat, src: Local, len: Local, idx: Local, elem: Local) -> Local {
	let new = w.local(types::valarray_ref());
	let dstoff = w.local(ValType::I32);
	let cnt = w.local(ValType::I32);
	w.local_get(len)
		.i32(1)
		.i32_add()
		.array_new_default(VA)
		.local_set(new);
	w.copy_loop(VA, new, None, src, None, idx); // [0..idx)
	w.local_get(new)
		.local_get(idx)
		.local_get(elem)
		.array_set(VA);
	w.local_get(idx).i32(1).i32_add().local_set(dstoff);
	w.local_get(len).local_get(idx).i32_sub().local_set(cnt);
	w.copy_loop(VA, new, Some(dstoff), src, Some(idx), cnt); // [idx..len) -> [idx+1..)
	new
}

/// Copy `src` (length `len`) with slot `idx` removed; return the new array
/// (length `len - 1`).
fn splice_remove(w: &mut Wat, src: Local, len: Local, idx: Local) -> Local {
	let new = w.local(types::valarray_ref());
	let srcoff = w.local(ValType::I32);
	let cnt = w.local(ValType::I32);
	w.local_get(len)
		.i32(1)
		.i32_sub()
		.array_new_default(VA)
		.local_set(new);
	w.copy_loop(VA, new, None, src, None, idx); // [0..idx)
	w.local_get(idx).i32(1).i32_add().local_set(srcoff);
	w.local_get(len)
		.i32(1)
		.i32_sub()
		.local_get(idx)
		.i32_sub()
		.local_set(cnt);
	w.copy_loop(VA, new, Some(idx), src, Some(srcoff), cnt); // [idx+1..len) -> [idx..)
	new
}

// ---------------------------------------------------------------------------
// `__hash` — a structural hash consistent with `__eq`. (Unchanged from the table
// representation: equal values hash equal; the trie keys on this.)
// ---------------------------------------------------------------------------

/// Build `__hash(value) -> $int`. FNV-1a over the value's structure, mirroring
/// `__eq`'s shape so equal values hash equal: tag, then the scalar payload or the
/// recursively-hashed children. `self_idx` is `__hash`'s own index (child
/// recursion). `ref`/`dict` keys (and any unhandled tag) collapse to the
/// tag-only hash — correct (`__eq` still separates them), just not finely
/// distributed; such keys are exotic.
pub(crate) fn build_hash_fn(
	self_idx: u32,
	variant_payload: u32,
	tuple_elems: u32,
	denom_idx: u32,
) -> Function {
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
	// A nominal `$shapeN` hashes as the uniform `$record` it lifts to — normalize
	// before the tag is mixed in, so the two forms (which `__eq` treats as equal)
	// produce the same hash.
	w.local_get(ta).i32(types::TAG_SHAPE).i32_eq();
	w.if_(|w| {
		w.local_get(v).call(denom_idx).local_set(v);
		w.i32(types::TAG_RECORD).local_set(ta);
	});
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
		w.local_get(v).call(variant_payload).local_set(arr);
		hash_elems(w, self_idx, arr, n, i, h, mix);
	});
	// TUPLE — mix each element's hash.
	w.local_get(ta).i32(types::TAG_TUPLE).i32_eq();
	w.if_(|w| {
		w.local_get(v).call(tuple_elems).local_set(arr);
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
// Construction / size / clear.
// ---------------------------------------------------------------------------

/// Build `__dict_empty(unit) -> $dict`: `{ tag, null root, 0 size }`. No allocation
/// for the (absent) root. The arg (unit) is ignored.
pub(crate) fn build_dict_empty_fn() -> Function {
	let mut w = Wat::new(1);
	w.i32(types::TAG_DICT);
	push_nothing(&mut w); // root = null
	w.i32(0); // size
	w.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_size(dict) -> $int`: the cached `size` field (O(1)).
pub(crate) fn build_dict_size_fn() -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.i64_extend_i32_u()
		.box_int();
	w.finish()
}

/// Build `__dict_clear(dict) -> $dict`: a fresh empty dict (value semantics — the
/// input is untouched). The arg is ignored.
pub(crate) fn build_dict_clear_fn() -> Function {
	let mut w = Wat::new(1);
	let _dict = w.param(0);
	w.i32(types::TAG_DICT);
	push_nothing(&mut w);
	w.i32(0);
	w.struct_new(types::T_DICT);
	w.finish()
}

// ---------------------------------------------------------------------------
// Recursive trie ops over `$cnode` (`hash`/`shift` are boxed ints).
// ---------------------------------------------------------------------------

/// Build `__cnode_lookup(node, key, hash, shift) -> $dentry|null`: descend from
/// `node`, returning the matching entry or null. `eq_idx` = `__eq`.
pub(crate) fn build_cnode_lookup_fn(self_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(4);
	let (node, key, bhash, bshift) = (w.param(0), w.param(1), w.param(2), w.param(3));
	let hash64 = w.local(ValType::I64);
	let shift64 = w.local(ValType::I64);
	let dmap = w.local(ValType::I32);
	let nmap = w.local(ValType::I32);
	let bit = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let entries = w.local(types::valarray_ref());
	let e = w.local(types::dentry_ref());
	let child = w.local(types::value_ref());

	// null node → absent.
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		push_nothing(w);
		w.ret();
	});
	w.local_get(bhash).unbox_int().local_set(hash64);
	w.local_get(bshift).unbox_int().local_set(shift64);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_DATAMAP)
		.local_set(dmap);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_NODEMAP)
		.local_set(nmap);
	// collision bucket (both maps 0) → linear scan of entries.
	w.local_get(dmap).local_get(nmap).i32_or().i32_eqz();
	w.if_(|w| {
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_ENTRIES)
			.local_set(entries);
		w.local_get(entries).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("bbrk", |w| {
			w.loop_("blp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("bbrk");
				w.local_get(entries)
					.local_get(i)
					.array_get(VA)
					.ref_cast(types::T_DENTRY)
					.local_set(e);
				w.local_get(e)
					.struct_get(types::T_DENTRY, DENTRY_KEY)
					.local_get(key)
					.call(eq_idx);
				w.if_(|w| {
					w.local_get(e).ret();
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("blp");
			});
		});
		push_nothing(w);
		w.ret();
	});
	// bit = 1 << chunk.
	w.i32(1);
	push_chunk(&mut w, hash64, shift64);
	w.i32_shl();
	w.local_set(bit);
	// data slot → the one leaf for this chunk; matches iff hash + key agree.
	w.local_get(dmap).local_get(bit).i32_and();
	w.if_(|w| {
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_ENTRIES)
			.local_set(entries);
		w.local_get(entries);
		push_idx(w, dmap, bit);
		w.array_get(VA).ref_cast(types::T_DENTRY).local_set(e);
		w.local_get(e)
			.struct_get(types::T_DENTRY, DENTRY_HASH)
			.local_get(hash64)
			.i64_eq();
		w.if_(|w| {
			w.local_get(e)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_get(key)
				.call(eq_idx);
			w.if_(|w| {
				w.local_get(e).ret();
			});
		});
		push_nothing(w);
		w.ret();
	});
	// node slot → recurse into the child with shift+5.
	w.local_get(nmap).local_get(bit).i32_and();
	w.if_(|w| {
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_CHILDREN);
		push_idx(w, nmap, bit);
		w.array_get(VA).local_set(child);
		w.local_get(child).local_get(key).local_get(bhash);
		w.local_get(shift64).i64(5).i64_add().box_int();
		w.call(self_idx);
		w.ret();
	});
	// empty slot → absent.
	push_nothing(&mut w);
	w.finish()
}

/// Build `__cnode_insert(node, key, val, hash, shift) -> node`: a path-copied node
/// with `key`→`val` set (a fresh single-leaf node when `node` is null). `eq_idx` =
/// `__eq`; `merge_idx` = `__cnode_merge`.
pub(crate) fn build_cnode_insert_fn(self_idx: u32, eq_idx: u32, merge_idx: u32) -> Function {
	let mut w = Wat::new(5);
	let (node, key, val, bhash, bshift) =
		(w.param(0), w.param(1), w.param(2), w.param(3), w.param(4));
	let hash64 = w.local(ValType::I64);
	let shift64 = w.local(ValType::I64);
	let zero = w.local(ValType::I32);
	let leaf = w.local(types::value_ref());
	let bshift5 = w.local(types::value_ref());
	let dmap = w.local(ValType::I32);
	let nmap = w.local(ValType::I32);
	let bit = w.local(ValType::I32);
	let di = w.local(ValType::I32);
	let ni = w.local(ValType::I32);
	let ne = w.local(ValType::I32);
	let nc = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let idx = w.local(ValType::I32);
	let newdmap = w.local(ValType::I32);
	let newnmap = w.local(ValType::I32);
	let entries = w.local(types::valarray_ref());
	let children = w.local(types::valarray_ref());
	let e = w.local(types::dentry_ref());
	let sub = w.local(types::value_ref());
	let child = w.local(types::value_ref());

	w.i32(0).local_set(zero);
	w.local_get(bhash).unbox_int().local_set(hash64);
	w.local_get(bshift).unbox_int().local_set(shift64);
	make_dentry(&mut w, key, val, hash64);
	w.local_set(leaf);
	w.local_get(shift64)
		.i64(5)
		.i64_add()
		.box_int()
		.local_set(bshift5);

	// null node → a fresh single-leaf node (dataMap = bit, entries = [leaf]).
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		w.i32(1);
		push_chunk(w, hash64, shift64);
		w.i32_shl();
		w.local_set(bit);
		let one = w.local(types::valarray_ref());
		let empty = w.local(types::valarray_ref());
		w.local_get(leaf).array_new_fixed(VA, 1).local_set(one);
		push_empty(w);
		w.local_set(empty);
		build_cnode(w, bit, zero, one, empty);
		w.ret();
	});
	// load bitmaps + compact arrays.
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_DATAMAP)
		.local_set(dmap);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_NODEMAP)
		.local_set(nmap);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_ENTRIES)
		.local_set(entries);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_CHILDREN)
		.local_set(children);
	w.local_get(entries).array_len().local_set(ne);
	w.local_get(children).array_len().local_set(nc);
	// collision bucket (both maps 0) → replace-or-append in the flat entries.
	w.local_get(dmap).local_get(nmap).i32_or().i32_eqz();
	w.if_(|w| {
		w.i32(-1).local_set(idx);
		w.i32(0).local_set(i);
		w.block("sbrk", |w| {
			w.loop_("slp", |w| {
				w.local_get(i).local_get(ne).i32_ge_s().br_if("sbrk");
				w.local_get(entries)
					.local_get(i)
					.array_get(VA)
					.ref_cast(types::T_DENTRY)
					.local_set(e);
				w.local_get(e)
					.struct_get(types::T_DENTRY, DENTRY_KEY)
					.local_get(key)
					.call(eq_idx);
				w.if_(|w| {
					w.local_get(i).local_set(idx);
					w.br("sbrk");
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("slp");
			});
		});
		w.local_get(idx).i32(0).i32_ge_s();
		w.if_(|w| {
			let r = copy_replace(w, entries, ne, idx, leaf);
			make_bucket(w, r);
			w.ret();
		});
		let a = splice_insert(w, entries, ne, ne, leaf);
		make_bucket(w, a);
		w.ret();
	});
	// bit = 1 << chunk.
	w.i32(1);
	push_chunk(&mut w, hash64, shift64);
	w.i32_shl();
	w.local_set(bit);
	// data slot → replace value (same key) or migrate both leaves into a sub-node.
	w.local_get(dmap).local_get(bit).i32_and();
	w.if_(|w| {
		push_idx(w, dmap, bit);
		w.local_set(di);
		w.local_get(entries)
			.local_get(di)
			.array_get(VA)
			.ref_cast(types::T_DENTRY)
			.local_set(e);
		w.local_get(e)
			.struct_get(types::T_DENTRY, DENTRY_KEY)
			.local_get(key)
			.call(eq_idx);
		w.if_(|w| {
			let r = copy_replace(w, entries, ne, di, leaf);
			build_cnode(w, dmap, nmap, r, children);
			w.ret();
		});
		// different key → merge the two leaves; move the slot from data to node.
		w.local_get(e)
			.local_get(leaf)
			.local_get(bshift5)
			.call(merge_idx)
			.local_set(sub);
		w.local_get(dmap)
			.local_get(bit)
			.i32_sub()
			.local_set(newdmap); // clear set data bit
		w.local_get(nmap).local_get(bit).i32_or().local_set(newnmap);
		let re = splice_remove(w, entries, ne, di);
		push_idx(w, nmap, bit);
		w.local_set(ni);
		let ic = splice_insert(w, children, nc, ni, sub);
		build_cnode(w, newdmap, newnmap, re, ic);
		w.ret();
	});
	// node slot → recurse, replace the child.
	w.local_get(nmap).local_get(bit).i32_and();
	w.if_(|w| {
		push_idx(w, nmap, bit);
		w.local_set(ni);
		w.local_get(children)
			.local_get(ni)
			.array_get(VA)
			.local_set(child);
		w.local_get(child)
			.local_get(key)
			.local_get(val)
			.local_get(bhash)
			.local_get(bshift5)
			.call(self_idx)
			.local_set(sub);
		let rc = copy_replace(w, children, nc, ni, sub);
		build_cnode(w, dmap, nmap, entries, rc);
		w.ret();
	});
	// empty slot → splice the leaf into entries, set its data bit.
	push_idx(&mut w, dmap, bit);
	w.local_set(di);
	w.local_get(dmap).local_get(bit).i32_or().local_set(newdmap);
	let ie = splice_insert(&mut w, entries, ne, di, leaf);
	build_cnode(&mut w, newdmap, nmap, ie, children);
	w.finish()
}

/// Build `__cnode_merge(dA, dB, shift) -> node`: the sub-node holding two
/// distinct-key leaves whose hashes agree up to `shift` (recursing while their
/// chunks collide; a flat bucket once the 64-bit hash is exhausted).
pub(crate) fn build_cnode_merge_fn(self_idx: u32) -> Function {
	let mut w = Wat::new(3);
	let (da, db, bshift) = (w.param(0), w.param(1), w.param(2));
	let shift64 = w.local(ValType::I64);
	let ha = w.local(ValType::I64);
	let hb = w.local(ValType::I64);
	let ca = w.local(ValType::I32);
	let cb = w.local(ValType::I32);
	let zero = w.local(ValType::I32);
	let bita = w.local(ValType::I32);
	let bitb = w.local(ValType::I32);
	let dmap = w.local(ValType::I32);
	let sub = w.local(types::value_ref());
	let entries = w.local(types::valarray_ref());
	let children = w.local(types::valarray_ref());
	let bshift5 = w.local(types::value_ref());

	w.i32(0).local_set(zero);
	w.local_get(bshift).unbox_int().local_set(shift64);
	// hash bits exhausted → a 2-entry collision bucket.
	w.local_get(shift64).i64(64).i64_ge_s();
	w.if_(|w| {
		w.local_get(da)
			.local_get(db)
			.array_new_fixed(VA, 2)
			.local_set(entries);
		make_bucket(w, entries);
		w.ret();
	});
	w.local_get(da)
		.ref_cast(types::T_DENTRY)
		.struct_get(types::T_DENTRY, DENTRY_HASH)
		.local_set(ha);
	w.local_get(db)
		.ref_cast(types::T_DENTRY)
		.struct_get(types::T_DENTRY, DENTRY_HASH)
		.local_set(hb);
	w.local_get(ha)
		.local_get(shift64)
		.i64_shr_u()
		.i32_wrap_i64()
		.i32(31)
		.i32_and()
		.local_set(ca);
	w.local_get(hb)
		.local_get(shift64)
		.i64_shr_u()
		.i32_wrap_i64()
		.i32(31)
		.i32_and()
		.local_set(cb);
	w.local_get(shift64)
		.i64(5)
		.i64_add()
		.box_int()
		.local_set(bshift5);
	// same chunk → one child node holding the deeper merge (nodeMap = 1<<ca).
	w.local_get(ca).local_get(cb).i32_eq();
	w.if_(|w| {
		w.local_get(da)
			.local_get(db)
			.local_get(bshift5)
			.call(self_idx)
			.local_set(sub);
		w.i32(1).local_get(ca).i32_shl().local_set(bita);
		w.local_get(sub).array_new_fixed(VA, 1).local_set(children);
		push_empty(w);
		w.local_set(entries);
		build_cnode(w, zero, bita, entries, children);
		w.ret();
	});
	// different chunks → two leaves in one node (dataMap = both bits), ordered by chunk.
	w.i32(1).local_get(ca).i32_shl().local_set(bita);
	w.i32(1).local_get(cb).i32_shl().local_set(bitb);
	w.local_get(bita).local_get(bitb).i32_or().local_set(dmap);
	// entries default [da, db]; overwrite to [db, da] when cb < ca (dominating set
	// before the branch, so the read after stays local-init valid).
	w.local_get(da)
		.local_get(db)
		.array_new_fixed(VA, 2)
		.local_set(entries);
	w.local_get(ca).local_get(cb).i32_gt_s();
	w.if_(|w| {
		w.local_get(db)
			.local_get(da)
			.array_new_fixed(VA, 2)
			.local_set(entries);
	});
	push_empty(&mut w);
	w.local_set(children);
	build_cnode(&mut w, dmap, zero, entries, children);
	w.finish()
}

/// Build `__cnode_remove(node, key, hash, shift) -> node`: a path-copied node with
/// `key` cleared (unchanged when absent — returns `node` as-is, sharing it). No
/// canonical re-compaction in the uncompressed HAMT layout: an emptied slot is left null.
/// `eq_idx` = `__eq`.
pub(crate) fn build_cnode_remove_fn(self_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(4);
	let (node, key, bhash, bshift) = (w.param(0), w.param(1), w.param(2), w.param(3));
	let hash64 = w.local(ValType::I64);
	let shift64 = w.local(ValType::I64);
	let dmap = w.local(ValType::I32);
	let nmap = w.local(ValType::I32);
	let bit = w.local(ValType::I32);
	let di = w.local(ValType::I32);
	let ni = w.local(ValType::I32);
	let ne = w.local(ValType::I32);
	let nc = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let idx = w.local(ValType::I32);
	let newdmap = w.local(ValType::I32);
	let entries = w.local(types::valarray_ref());
	let children = w.local(types::valarray_ref());
	let e = w.local(types::dentry_ref());
	let child = w.local(types::value_ref());
	let newsub = w.local(types::value_ref());
	let bshift5 = w.local(types::value_ref());

	w.local_get(bhash).unbox_int().local_set(hash64);
	w.local_get(bshift).unbox_int().local_set(shift64);
	w.local_get(shift64)
		.i64(5)
		.i64_add()
		.box_int()
		.local_set(bshift5);

	// null node → null (the wrapper guards, but stay total).
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		push_nothing(w);
		w.ret();
	});
	// load bitmaps + compact arrays.
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_DATAMAP)
		.local_set(dmap);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_NODEMAP)
		.local_set(nmap);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_ENTRIES)
		.local_set(entries);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_CHILDREN)
		.local_set(children);
	w.local_get(entries).array_len().local_set(ne);
	w.local_get(children).array_len().local_set(nc);
	// collision bucket → splice out the matching entry (or unchanged).
	w.local_get(dmap).local_get(nmap).i32_or().i32_eqz();
	w.if_(|w| {
		w.i32(-1).local_set(idx);
		w.i32(0).local_set(i);
		w.block("rbrk", |w| {
			w.loop_("rlp", |w| {
				w.local_get(i).local_get(ne).i32_ge_s().br_if("rbrk");
				w.local_get(entries)
					.local_get(i)
					.array_get(VA)
					.ref_cast(types::T_DENTRY)
					.local_set(e);
				w.local_get(e)
					.struct_get(types::T_DENTRY, DENTRY_KEY)
					.local_get(key)
					.call(eq_idx);
				w.if_(|w| {
					w.local_get(i).local_set(idx);
					w.br("rbrk");
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("rlp");
			});
		});
		// not found → unchanged.
		w.local_get(idx).i32(0).i32_lt_s();
		w.if_(|w| {
			w.local_get(node).ret();
		});
		let r = splice_remove(w, entries, ne, idx);
		make_bucket(w, r);
		w.ret();
	});
	// bit = 1 << chunk.
	w.i32(1);
	push_chunk(&mut w, hash64, shift64);
	w.i32_shl();
	w.local_set(bit);
	// data slot → remove the leaf if it's our key (clear its data bit), else unchanged.
	w.local_get(dmap).local_get(bit).i32_and();
	w.if_(|w| {
		push_idx(w, dmap, bit);
		w.local_set(di);
		w.local_get(entries)
			.local_get(di)
			.array_get(VA)
			.ref_cast(types::T_DENTRY)
			.local_set(e);
		w.local_get(e)
			.struct_get(types::T_DENTRY, DENTRY_KEY)
			.local_get(key)
			.call(eq_idx);
		w.if_(|w| {
			let r = splice_remove(w, entries, ne, di);
			w.local_get(dmap)
				.local_get(bit)
				.i32_sub()
				.local_set(newdmap);
			build_cnode(w, newdmap, nmap, r, children);
			w.ret();
		});
		w.local_get(node).ret();
	});
	// node slot → recurse, replace the child (an emptied child is left in place — it
	// reads back as an empty bucket → absent, so no canonicalization is needed).
	w.local_get(nmap).local_get(bit).i32_and();
	w.if_(|w| {
		push_idx(w, nmap, bit);
		w.local_set(ni);
		w.local_get(children)
			.local_get(ni)
			.array_get(VA)
			.local_set(child);
		w.local_get(child)
			.local_get(key)
			.local_get(bhash)
			.local_get(bshift5)
			.call(self_idx)
			.local_set(newsub);
		let rc = copy_replace(w, children, nc, ni, newsub);
		build_cnode(w, dmap, nmap, entries, rc);
		w.ret();
	});
	// empty slot → unchanged.
	w.local_get(node);
	w.finish()
}

/// Build `__cnode_collect(node, list) -> nothing`: append every `(key, value)`
/// under `node` to `list` (in-place `__list_push`), recursing into sub-nodes.
/// `push_idx` = `__list_push`.
pub(crate) fn build_cnode_collect_fn(self_idx: u32, push_fn: u32) -> Function {
	let mut w = Wat::new(2);
	let (node, list) = (w.param(0), w.param(1));
	let entries = w.local(types::valarray_ref());
	let children = w.local(types::valarray_ref());
	let ne = w.local(ValType::I32);
	let nc = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let e = w.local(types::dentry_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());
	let child = w.local(types::value_ref());

	// push every leaf (entries), then recurse into every child.
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_ENTRIES)
		.local_set(entries);
	w.local_get(entries).array_len().local_set(ne);
	w.i32(0).local_set(i);
	w.block("ebrk", |w| {
		w.loop_("elp", |w| {
			w.local_get(i).local_get(ne).i32_ge_s().br_if("ebrk");
			w.local_get(entries)
				.local_get(i)
				.array_get(VA)
				.ref_cast(types::T_DENTRY)
				.local_set(e);
			w.local_get(e)
				.struct_get(types::T_DENTRY, DENTRY_KEY)
				.local_set(k);
			w.local_get(e)
				.struct_get(types::T_DENTRY, DENTRY_VAL)
				.local_set(v);
			w.local_get(list);
			tuple2(w, k, v);
			w.call(push_fn).drop();
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("elp");
		});
	});
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_CHILDREN)
		.local_set(children);
	w.local_get(children).array_len().local_set(nc);
	w.i32(0).local_set(i);
	w.block("cbrk", |w| {
		w.loop_("clp", |w| {
			w.local_get(i).local_get(nc).i32_ge_s().br_if("cbrk");
			w.local_get(children)
				.local_get(i)
				.array_get(VA)
				.local_set(child);
			w.local_get(child).local_get(list).call(self_idx).drop();
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("clp");
		});
	});
	push_nothing(&mut w);
	w.finish()
}

// ---------------------------------------------------------------------------
// Transient build path (internal): in-place mutation of session-owned nodes.
// ---------------------------------------------------------------------------

/// Build `__cnode_tinsert(node, key, val, hash, shift, token) -> node`: like
/// `__cnode_insert`, but mutates a node *in place* when it is owned by the current
/// transient `token` (`ref.eq(node.edit, token)`). A node that isn't owned (null,
/// or frozen / another session's) is first turned into an owned one — a fresh
/// stamped node for null, else a copy-on-write of the struct **and both arrays**
/// stamped with `token` — so a frozen dict shared elsewhere is never mutated. Only
/// the stdlib builders call this, each with its own fresh token. `merge` stays
/// persistent; tinsert COW-copies any frozen sub-node it later re-touches.
pub(crate) fn build_cnode_tinsert_fn(self_idx: u32, eq_idx: u32, merge_idx: u32) -> Function {
	let mut w = Wat::new(6);
	let (node, key, val, bhash, bshift, token) = (
		w.param(0),
		w.param(1),
		w.param(2),
		w.param(3),
		w.param(4),
		w.param(5),
	);
	let hash64 = w.local(ValType::I64);
	let shift64 = w.local(ValType::I64);
	let zero = w.local(ValType::I32);
	let leaf = w.local(types::value_ref());
	let bshift5 = w.local(types::value_ref());
	let nd = w.local(types::cnode_ref());
	let dmap = w.local(ValType::I32);
	let nmap = w.local(ValType::I32);
	let bit = w.local(ValType::I32);
	let di = w.local(ValType::I32);
	let ni = w.local(ValType::I32);
	let ne = w.local(ValType::I32);
	let nc = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let idx = w.local(ValType::I32);
	let entries = w.local(types::valarray_ref());
	let children = w.local(types::valarray_ref());
	let e = w.local(types::dentry_ref());
	let sub = w.local(types::value_ref());
	let child = w.local(types::value_ref());
	let nc2 = w.local(types::value_ref());

	w.i32(0).local_set(zero);
	w.local_get(bhash).unbox_int().local_set(hash64);
	w.local_get(bshift).unbox_int().local_set(shift64);
	make_dentry(&mut w, key, val, hash64);
	w.local_set(leaf);
	w.local_get(shift64)
		.i64(5)
		.i64_add()
		.box_int()
		.local_set(bshift5);

	// null node → a fresh single-leaf node stamped with the token.
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		w.i32(1);
		push_chunk(w, hash64, shift64);
		w.i32_shl();
		w.local_set(bit);
		let one = w.local(types::valarray_ref());
		let empty = w.local(types::valarray_ref());
		w.local_get(leaf).array_new_fixed(VA, 1).local_set(one);
		push_empty(w);
		w.local_set(empty);
		build_cnode_t(w, bit, zero, one, empty, token);
		w.ret();
	});
	// Obtain an OWNED node `nd`: default to `node`; if it isn't ours, COW-copy it
	// (struct + both arrays) and stamp the copy. (Dominating set before the branch
	// keeps the later read local-init valid.)
	w.local_get(node).ref_cast(types::T_CNODE).local_set(nd);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_EDIT)
		.local_get(token)
		.ref_eq()
		.i32_eqz();
	w.if_(|w| {
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_DATAMAP)
			.local_set(dmap);
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_NODEMAP)
			.local_set(nmap);
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_ENTRIES)
			.local_set(entries);
		w.local_get(node)
			.ref_cast(types::T_CNODE)
			.struct_get(types::T_CNODE, CN_CHILDREN)
			.local_set(children);
		w.local_get(entries).array_len().local_set(ne);
		w.local_get(children).array_len().local_set(nc);
		let ce = copy_all(w, entries, ne);
		let cc = copy_all(w, children, nc);
		build_cnode_t(w, dmap, nmap, ce, cc, token);
		w.local_set(nd);
	});
	// Load nd's (owned) bitmaps + arrays — safe to mutate in place.
	w.local_get(nd)
		.struct_get(types::T_CNODE, CN_DATAMAP)
		.local_set(dmap);
	w.local_get(nd)
		.struct_get(types::T_CNODE, CN_NODEMAP)
		.local_set(nmap);
	w.local_get(nd)
		.struct_get(types::T_CNODE, CN_ENTRIES)
		.local_set(entries);
	w.local_get(nd)
		.struct_get(types::T_CNODE, CN_CHILDREN)
		.local_set(children);
	w.local_get(entries).array_len().local_set(ne);
	w.local_get(children).array_len().local_set(nc);
	// collision bucket (both maps 0) → in-place replace, or grow + reassign.
	w.local_get(dmap).local_get(nmap).i32_or().i32_eqz();
	w.if_(|w| {
		w.i32(-1).local_set(idx);
		w.i32(0).local_set(i);
		w.block("sbrk", |w| {
			w.loop_("slp", |w| {
				w.local_get(i).local_get(ne).i32_ge_s().br_if("sbrk");
				w.local_get(entries)
					.local_get(i)
					.array_get(VA)
					.ref_cast(types::T_DENTRY)
					.local_set(e);
				w.local_get(e)
					.struct_get(types::T_DENTRY, DENTRY_KEY)
					.local_get(key)
					.call(eq_idx);
				w.if_(|w| {
					w.local_get(i).local_set(idx);
					w.br("sbrk");
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("slp");
			});
		});
		w.local_get(idx).i32(0).i32_ge_s();
		w.if_(|w| {
			w.local_get(entries)
				.local_get(idx)
				.local_get(leaf)
				.array_set(VA);
			w.local_get(nd).ret();
		});
		let a = splice_insert(w, entries, ne, ne, leaf);
		w.local_get(nd)
			.local_get(a)
			.struct_set(types::T_CNODE, CN_ENTRIES);
		w.local_get(nd).ret();
	});
	// bit = 1 << chunk.
	w.i32(1);
	push_chunk(&mut w, hash64, shift64);
	w.i32_shl();
	w.local_set(bit);
	// data slot → in-place value replace, or migrate the two leaves into a sub-node.
	w.local_get(dmap).local_get(bit).i32_and();
	w.if_(|w| {
		push_idx(w, dmap, bit);
		w.local_set(di);
		w.local_get(entries)
			.local_get(di)
			.array_get(VA)
			.ref_cast(types::T_DENTRY)
			.local_set(e);
		w.local_get(e)
			.struct_get(types::T_DENTRY, DENTRY_KEY)
			.local_get(key)
			.call(eq_idx);
		w.if_(|w| {
			w.local_get(entries)
				.local_get(di)
				.local_get(leaf)
				.array_set(VA);
			w.local_get(nd).ret();
		});
		w.local_get(e)
			.local_get(leaf)
			.local_get(bshift5)
			.call(merge_idx)
			.local_set(sub);
		let re = splice_remove(w, entries, ne, di);
		w.local_get(nd)
			.local_get(re)
			.struct_set(types::T_CNODE, CN_ENTRIES);
		push_idx(w, nmap, bit);
		w.local_set(ni);
		let ic = splice_insert(w, children, nc, ni, sub);
		w.local_get(nd)
			.local_get(ic)
			.struct_set(types::T_CNODE, CN_CHILDREN);
		w.local_get(nd)
			.local_get(dmap)
			.local_get(bit)
			.i32_sub()
			.struct_set(types::T_CNODE, CN_DATAMAP);
		w.local_get(nd)
			.local_get(nmap)
			.local_get(bit)
			.i32_or()
			.struct_set(types::T_CNODE, CN_NODEMAP);
		w.local_get(nd).ret();
	});
	// node slot → recurse transiently, write the (possibly copied) child back.
	w.local_get(nmap).local_get(bit).i32_and();
	w.if_(|w| {
		push_idx(w, nmap, bit);
		w.local_set(ni);
		w.local_get(children)
			.local_get(ni)
			.array_get(VA)
			.local_set(child);
		w.local_get(child)
			.local_get(key)
			.local_get(val)
			.local_get(bhash)
			.local_get(bshift5)
			.local_get(token)
			.call(self_idx)
			.local_set(nc2);
		w.local_get(children)
			.local_get(ni)
			.local_get(nc2)
			.array_set(VA);
		w.local_get(nd).ret();
	});
	// empty slot → splice the leaf into entries + set its data bit, in place.
	push_idx(&mut w, dmap, bit);
	w.local_set(di);
	let ie = splice_insert(&mut w, entries, ne, di, leaf);
	w.local_get(nd)
		.local_get(ie)
		.struct_set(types::T_CNODE, CN_ENTRIES);
	w.local_get(nd)
		.local_get(dmap)
		.local_get(bit)
		.i32_or()
		.struct_set(types::T_CNODE, CN_DATAMAP);
	w.local_get(nd);
	w.finish()
}

/// Build `__cnode_count(node) -> $int`: the number of leaves under `node` (entries
/// here, plus each child's count). Backs `__dict_from_entries`'s `size` (duplicates
/// in the input collapse, so the live count isn't known until the tree is built).
pub(crate) fn build_cnode_count_fn(self_idx: u32) -> Function {
	let mut w = Wat::new(1);
	let node = w.param(0);
	let entries = w.local(types::valarray_ref());
	let children = w.local(types::valarray_ref());
	let nc = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let total = w.local(ValType::I64);

	// null → 0.
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		w.i64(0).box_int();
		w.ret();
	});
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_ENTRIES)
		.local_set(entries);
	w.local_get(entries)
		.array_len()
		.i64_extend_i32_u()
		.local_set(total);
	w.local_get(node)
		.ref_cast(types::T_CNODE)
		.struct_get(types::T_CNODE, CN_CHILDREN)
		.local_set(children);
	w.local_get(children).array_len().local_set(nc);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(nc).i32_ge_s().br_if("brk");
			w.local_get(total);
			w.local_get(children)
				.local_get(i)
				.array_get(VA)
				.call(self_idx)
				.unbox_int();
			w.i64_add().local_set(total);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(total).box_int();
	w.finish()
}

/// Build `__dict_from_entries(list (k, v)) -> $dict`: build the dict transiently —
/// one fresh owner token, `tinsert` each pair (later pairs win), then `count` the
/// result for `size`. The transient mutates only its own session-owned nodes, so
/// the returned dict is a normal immutable value. `hash_idx`/`tinsert_idx`/`count_idx`
/// = `__hash`/`__cnode_tinsert`/`__cnode_count`.
pub(crate) fn build_dict_from_entries_fn(
	hash_idx: u32,
	tinsert_idx: u32,
	count_idx: u32,
) -> Function {
	let mut w = Wat::new(1);
	let lst = w.param(0);
	let token = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let tup = w.local(types::value_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());
	let size = w.local(ValType::I32);

	push_token(&mut w);
	w.local_set(token);
	w.i32(0).ref_i31().local_set(b0);
	push_nothing(&mut w);
	w.local_set(root);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(n);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(elems).local_get(i).array_get(VA).local_set(tup);
			// entry is a `(k, v)` pair — read the inline slots (fields 2, 3).
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 2)
				.local_set(k);
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 3)
				.local_set(v);
			// root = tinsert(root, k, v, hash(k), 0, token).
			w.local_get(root).local_get(k).local_get(v);
			w.local_get(k).call(hash_idx);
			w.local_get(b0).local_get(token);
			w.call(tinsert_idx).local_set(root);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(root)
		.call(count_idx)
		.unbox_int()
		.i32_wrap_i64()
		.local_set(size);
	w.i32(types::TAG_DICT)
		.local_get(root)
		.local_get(size)
		.struct_new(types::T_DICT);
	w.finish()
}

// ---------------------------------------------------------------------------
// Public wrappers — compute the key hash once, then drive the trie ops.
// ---------------------------------------------------------------------------

/// Build `__dict_find(dict, key) -> $dentry|null`: hash the key, walk from the root.
/// `hash_idx` = `__hash`; `cnlookup_idx` = `__cnode_lookup`.
pub(crate) fn build_dict_find_fn(hash_idx: u32, cnlookup_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let bhash = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());

	w.local_get(key).call(hash_idx).local_set(bhash);
	w.i32(0).ref_i31().local_set(b0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ROOT)
		.local_set(root);
	w.local_get(root)
		.local_get(key)
		.local_get(bhash)
		.local_get(b0)
		.call(cnlookup_idx);
	w.finish()
}

/// Build `__dict_lookup(dict, key) -> option value`: `__dict_find` then wrap in
/// `some`/`none`. `find_idx` = `__dict_find`; `opt` builds the variant literals.
pub(crate) fn build_dict_lookup_fn(find_idx: u32, opt: OptionLits) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let e = w.local(types::value_ref());

	w.local_get(dict).local_get(key).call(find_idx).local_set(e);
	w.local_get(e).ref_is_null();
	w.if_(|w| {
		build_none(w, opt);
		w.ret();
	});
	start_some(&mut w, opt);
	w.local_get(e)
		.ref_cast(types::T_DENTRY)
		.struct_get(types::T_DENTRY, DENTRY_VAL);
	finish_some(&mut w);
	w.finish()
}

/// Build `__dict_insert(dict, key, val) -> $dict`: a new dict with `key`→`val`.
/// Looks the key up first (to know whether `size` grows), then path-copies.
/// `hash_idx`/`cnlookup_idx`/`cninsert_idx` = `__hash`/`__cnode_lookup`/`__cnode_insert`.
pub(crate) fn build_dict_insert_fn(
	hash_idx: u32,
	cnlookup_idx: u32,
	cninsert_idx: u32,
) -> Function {
	let mut w = Wat::new(3);
	let (dict, key, val) = (w.param(0), w.param(1), w.param(2));
	let bhash = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let found = w.local(types::value_ref());
	let delta = w.local(ValType::I32);
	let newroot = w.local(types::value_ref());
	let size = w.local(ValType::I32);

	w.local_get(key).call(hash_idx).local_set(bhash);
	w.i32(0).ref_i31().local_set(b0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ROOT)
		.local_set(root);
	w.local_get(root)
		.local_get(key)
		.local_get(bhash)
		.local_get(b0)
		.call(cnlookup_idx)
		.local_set(found);
	// delta = 1 when the key was absent (ref_is_null → 1).
	w.local_get(found).ref_is_null().local_set(delta);
	w.local_get(root)
		.local_get(key)
		.local_get(val)
		.local_get(bhash)
		.local_get(b0)
		.call(cninsert_idx)
		.local_set(newroot);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.local_get(delta)
		.i32_add()
		.local_set(size);
	w.i32(types::TAG_DICT)
		.local_get(newroot)
		.local_get(size)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_mint_token(unit?) -> $value`: a fresh transient owner token (a
/// bare `$value`, compared only by `ref.eq`). Minted once at the start of a linear
/// dict region by the reuse pass and threaded into every
/// `__dict_insert_into` in that region.
pub(crate) fn build_dict_mint_token_fn() -> Function {
	let mut w = Wat::new(0);
	push_token(&mut w);
	w.finish()
}

/// Build `__dict_insert_into(dict, key, val, token) -> $dict`: like `__dict_insert`
/// but transient — it threads `token` into `__cnode_tinsert`, which mutates in place
/// any node owned by `token` and copy-on-writes any node that isn't (so a frozen or
/// foreign dict is never corrupted). The reuse pass rewrites a `dict.insert` to this
/// only when it has proven the input dict is uniquely owned and dead after the
/// insert, so the in-place mutation is unobservable. `size` is maintained exactly as
/// `__dict_insert` does (a pre-lookup gives the add-vs-replace delta).
/// `hash_idx`/`cnlookup_idx`/`cntinsert_idx` = `__hash`/`__cnode_lookup`/`__cnode_tinsert`.
pub(crate) fn build_dict_insert_into_fn(
	hash_idx: u32,
	cnlookup_idx: u32,
	cntinsert_idx: u32,
) -> Function {
	let mut w = Wat::new(4);
	let (dict, key, val, token) = (w.param(0), w.param(1), w.param(2), w.param(3));
	let bhash = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let found = w.local(types::value_ref());
	let delta = w.local(ValType::I32);
	let newroot = w.local(types::value_ref());
	let size = w.local(ValType::I32);

	w.local_get(key).call(hash_idx).local_set(bhash);
	w.i32(0).ref_i31().local_set(b0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ROOT)
		.local_set(root);
	w.local_get(root)
		.local_get(key)
		.local_get(bhash)
		.local_get(b0)
		.call(cnlookup_idx)
		.local_set(found);
	// delta = 1 when the key was absent (ref_is_null → 1).
	w.local_get(found).ref_is_null().local_set(delta);
	w.local_get(root)
		.local_get(key)
		.local_get(val)
		.local_get(bhash)
		.local_get(b0)
		.local_get(token)
		.call(cntinsert_idx)
		.local_set(newroot);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.local_get(delta)
		.i32_add()
		.local_set(size);
	w.i32(types::TAG_DICT)
		.local_get(newroot)
		.local_get(size)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_remove(dict, key) -> $dict`: a new dict without `key` (the input
/// dict, unchanged, when the key is absent). `cnremove_idx` = `__cnode_remove`.
pub(crate) fn build_dict_remove_fn(
	hash_idx: u32,
	cnlookup_idx: u32,
	cnremove_idx: u32,
) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let bhash = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let found = w.local(types::value_ref());
	let newroot = w.local(types::value_ref());
	let size = w.local(ValType::I32);

	w.local_get(key).call(hash_idx).local_set(bhash);
	w.i32(0).ref_i31().local_set(b0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ROOT)
		.local_set(root);
	w.local_get(root)
		.local_get(key)
		.local_get(bhash)
		.local_get(b0)
		.call(cnlookup_idx)
		.local_set(found);
	// absent → unchanged.
	w.local_get(found).ref_is_null();
	w.if_(|w| {
		w.local_get(dict).ret();
	});
	w.local_get(root)
		.local_get(key)
		.local_get(bhash)
		.local_get(b0)
		.call(cnremove_idx)
		.local_set(newroot);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.i32(1)
		.i32_sub()
		.local_set(size);
	w.i32(types::TAG_DICT)
		.local_get(newroot)
		.local_get(size)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_update(dict, key, f) -> $dict`: a fused read-modify-write. Calls
/// `f` with `some(current)`/`none` and inserts its result. `arity1` is `f`'s
/// `(env, option v)` indirect type; `opt` builds the some/none argument.
pub(crate) fn build_dict_update_fn(
	hash_idx: u32,
	cnlookup_idx: u32,
	cninsert_idx: u32,
	arity1: u32,
	opt: OptionLits,
) -> Function {
	let mut w = Wat::new(3);
	let (dict, key, f) = (w.param(0), w.param(1), w.param(2));
	let bhash = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let found = w.local(types::value_ref());
	let arg = w.local(types::value_ref());
	let newval = w.local(types::value_ref());
	let newroot = w.local(types::value_ref());
	let delta = w.local(ValType::I32);
	let size = w.local(ValType::I32);

	w.local_get(key).call(hash_idx).local_set(bhash);
	w.i32(0).ref_i31().local_set(b0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ROOT)
		.local_set(root);
	w.local_get(root)
		.local_get(key)
		.local_get(bhash)
		.local_get(b0)
		.call(cnlookup_idx)
		.local_set(found);
	// arg = some(found.value) / none; delta = 1 when absent.
	w.local_get(found).ref_is_null();
	w.if_else(
		|w| {
			build_none(w, opt);
			w.local_set(arg);
			w.i32(1).local_set(delta);
		},
		|w| {
			start_some(w, opt);
			w.local_get(found)
				.ref_cast(types::T_DENTRY)
				.struct_get(types::T_DENTRY, DENTRY_VAL);
			finish_some(w);
			w.local_set(arg);
			w.i32(0).local_set(delta);
		},
	);
	// newval = f(arg).
	w.local_get(f).ref_cast(types::T_CLOSURE);
	w.local_get(arg);
	w.local_get(f)
		.ref_cast(types::T_CLOSURE)
		.struct_get(types::T_CLOSURE, CLOSURE_FN);
	w.call_indirect(arity1);
	w.local_set(newval);
	w.local_get(root)
		.local_get(key)
		.local_get(newval)
		.local_get(bhash)
		.local_get(b0)
		.call(cninsert_idx)
		.local_set(newroot);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.local_get(delta)
		.i32_add()
		.local_set(size);
	w.i32(types::TAG_DICT)
		.local_get(newroot)
		.local_get(size)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_entries(dict) -> list (k, v)`: walk the trie collecting every
/// `(key, value)` tuple (hash order). `collect_idx` = `__cnode_collect`.
pub(crate) fn build_dict_entries_fn(collect_idx: u32) -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	let list = w.local(types::value_ref());
	let root = w.local(types::value_ref());

	// list = empty $list { tag, [], 0 }.
	w.i32(types::TAG_LIST);
	w.i32(0).array_new_default(VA);
	w.i32(0);
	w.struct_new(types::T_LIST);
	w.local_set(list);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_ROOT)
		.local_set(root);
	w.local_get(root).ref_is_null().i32_eqz();
	w.if_(|w| {
		w.local_get(root).local_get(list).call(collect_idx).drop();
	});
	w.local_get(list);
	w.finish()
}

/// Build `__dict_eq(a, b) -> i32`: equal iff same size and every entry of `a` is in
/// `b` with an `__eq` value. `eq_idx`/`find_idx`/`entries_idx` =
/// `__eq`/`__dict_find`/`__dict_entries`.
pub(crate) fn build_dict_eq_fn(eq_idx: u32, find_idx: u32, entries_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let la = w.local(ValType::I32);
	let lb = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let es = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let tup = w.local(types::value_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());
	let eb = w.local(types::value_ref());

	w.local_get(a)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.local_set(la);
	w.local_get(b)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, DICT_SIZE)
		.local_set(lb);
	w.local_get(la).local_get(lb).i32_ne();
	w.if_(|w| {
		w.i32(0).ret();
	});
	w.local_get(a).call(entries_idx).local_set(es);
	w.local_get(es)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(es)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(n);
	w.i32(0).local_set(i);
	w.block("ebrk", |w| {
		w.loop_("elp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("ebrk");
			w.local_get(elems).local_get(i).array_get(VA).local_set(tup);
			// entry is a `(k, v)` pair — read the inline slots (fields 2, 3).
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 2)
				.local_set(k);
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 3)
				.local_set(v);
			// eb = find(b, k); absent → unequal.
			w.local_get(b).local_get(k).call(find_idx).local_set(eb);
			w.local_get(eb).ref_is_null();
			w.if_(|w| {
				w.i32(0).ret();
			});
			// values must be __eq.
			w.local_get(v)
				.local_get(eb)
				.ref_cast(types::T_DENTRY)
				.struct_get(types::T_DENTRY, DENTRY_VAL)
				.call(eq_idx)
				.i32_eqz();
			w.if_(|w| {
				w.i32(0).ret();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("elp");
		});
	});
	w.i32(1);
	w.finish()
}

/// Build `__dict_map(dict, f) -> dict`: a fresh dict with `f` applied to each value,
/// built transiently (keys are preserved + distinct, so `size` is the entry count).
/// `hash_idx`/`tinsert_idx`/`entries_idx` = `__hash`/`__cnode_tinsert`/`__dict_entries`;
/// `arity1` is `f`'s `(env, value)` indirect type.
pub(crate) fn build_dict_map_fn(
	hash_idx: u32,
	tinsert_idx: u32,
	entries_idx: u32,
	arity1: u32,
) -> Function {
	let mut w = Wat::new(2);
	let (dict, f) = (w.param(0), w.param(1));
	let token = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let es = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let tup = w.local(types::value_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());
	let nv = w.local(types::value_ref());

	push_token(&mut w);
	w.local_set(token);
	w.i32(0).ref_i31().local_set(b0);
	push_nothing(&mut w);
	w.local_set(root);
	w.local_get(dict).call(entries_idx).local_set(es);
	w.local_get(es)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(es)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(n);
	w.i32(0).local_set(i);
	w.block("mbrk", |w| {
		w.loop_("mlp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("mbrk");
			w.local_get(elems).local_get(i).array_get(VA).local_set(tup);
			// entry is a `(k, v)` pair — read the inline slots (fields 2, 3).
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 2)
				.local_set(k);
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 3)
				.local_set(v);
			// nv = f(v).
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(v);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, CLOSURE_FN);
			w.call_indirect(arity1);
			w.local_set(nv);
			// root = tinsert(root, k, nv, hash(k), 0, token).
			w.local_get(root).local_get(k).local_get(nv);
			w.local_get(k).call(hash_idx);
			w.local_get(b0).local_get(token);
			w.call(tinsert_idx).local_set(root);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("mlp");
		});
	});
	w.i32(types::TAG_DICT)
		.local_get(root)
		.local_get(n)
		.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_filter(dict, f) -> dict`: a fresh dict of the entries where `f k v`
/// is true, built transiently (a kept-counter gives `size`). `arity2` is `f`'s
/// `(env, key, value)` indirect type.
pub(crate) fn build_dict_filter_fn(
	hash_idx: u32,
	tinsert_idx: u32,
	entries_idx: u32,
	arity2: u32,
) -> Function {
	let mut w = Wat::new(2);
	let (dict, f) = (w.param(0), w.param(1));
	let token = w.local(types::value_ref());
	let b0 = w.local(types::value_ref());
	let root = w.local(types::value_ref());
	let es = w.local(types::value_ref());
	let elems = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let cnt = w.local(ValType::I32);
	let tup = w.local(types::value_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());

	push_token(&mut w);
	w.local_set(token);
	w.i32(0).ref_i31().local_set(b0);
	push_nothing(&mut w);
	w.local_set(root);
	w.i32(0).local_set(cnt);
	w.local_get(dict).call(entries_idx).local_set(es);
	w.local_get(es)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_ELEMS)
		.local_set(elems);
	w.local_get(es)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, LIST_LEN)
		.local_set(n);
	w.i32(0).local_set(i);
	w.block("fbrk", |w| {
		w.loop_("flp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("fbrk");
			w.local_get(elems).local_get(i).array_get(VA).local_set(tup);
			// entry is a `(k, v)` pair — read the inline slots (fields 2, 3).
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 2)
				.local_set(k);
			w.local_get(tup)
				.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 3)
				.local_set(v);
			// keep = f(k, v).
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(k).local_get(v);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, CLOSURE_FN);
			w.call_indirect(arity2);
			w.ref_cast(types::T_BOOL).struct_get(types::T_BOOL, 1);
			w.if_(|w| {
				// root = tinsert(root, k, v, hash(k), 0, token); cnt += 1.
				w.local_get(root).local_get(k).local_get(v);
				w.local_get(k).call(hash_idx);
				w.local_get(b0).local_get(token);
				w.call(tinsert_idx).local_set(root);
				w.local_get(cnt).i32(1).i32_add().local_set(cnt);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("flp");
		});
	});
	w.i32(types::TAG_DICT)
		.local_get(root)
		.local_get(cnt)
		.struct_new(types::T_DICT);
	w.finish()
}
