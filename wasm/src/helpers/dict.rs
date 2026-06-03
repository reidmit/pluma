// `core.dict` as a persistent hash-trie (a persistent, structurally-shared
// immutable map). The old representation was an insertion-ordered association array that
// linear-scanned with `__eq` and full-copied on every insert — O(n) per insert,
// O(n²) to build. This replaces it with a 16-way array-mapped trie keyed by a
// structural hash (`__hash`), so insert/lookup are O(log₁₆ n) and an immutable
// insert path-copies only the O(log n) nodes it touches (every other subtree is
// shared by reference — the GC handles lifetimes).
//
// Layout (see `types.rs`): a `$dict` is `{ tag, root, next_seq }`. `root` is a
// `$dnode` trie node (or null when empty). A `$dnode` is `{ tag, kids, ents,
// leafhash }`:
//
//   * branch — `kids` is a 16-slot `$valarray` of child `$dnode`s (null = absent),
//     `ents` null. The nibble `hash & 0xF` (the hash is shifted right by 4 as we
//     descend) selects a child.
//   * leaf — `ents` is a `$valarray` of `$tuple(key, value, seq)` entries that all
//     share `leafhash` (the hash bits unconsumed at this depth), `kids` null. A
//     leaf splits into a branch when an insert arrives whose remaining hash
//     differs from `leafhash`; entries that share a full hash (a true collision)
//     stay together in one leaf's bucket and are told apart by `__eq`.
//
// `seq` is a per-entry insertion stamp (the dict's `next_seq` at insert time) so
// `dict.entries`/`keys`/`values` can recover insertion order. `dict.size` is an
// O(n) trie count — size isn't stored, since it's read far less often than
// insert/lookup run. remove/map/filter are expressed as "materialize sorted
// entries, transform, rebuild via insert", so only insert/lookup/collect/count
// actually walk the trie.

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::{Local, Wat};
use crate::runtime::OptionLits;
use crate::types;

// FNV-1a 64-bit constants. `__hash` need only be *consistent with `__eq`* (equal
// values hash equal); the exact mixing is internal, so these match no other
// component — they're just a well-distributed standard hash.
const FNV_OFFSET: i64 = 0xcbf2_9ce4_8422_2325u64 as i64;
const FNV_PRIME: i64 = 0x0000_0100_0000_01b3;

// Branching factor: 16-way (4 hash bits per level), so a 64-bit hash bottoms out
// after at most 16 levels with no rebalancing.
const FANOUT: u32 = 16;
const NIBBLE_MASK: i64 = 0xF;
const NIBBLE_BITS: i64 = 4;

const VA: u32 = types::T_VALARRAY;

// ---------------------------------------------------------------------------
// Small emitters shared across the trie helpers.
// ---------------------------------------------------------------------------

/// Push a fresh leaf `$dnode { tag, kids: null, ents, leafhash }`.
fn make_leaf(w: &mut Wat, ents: Local, leafhash: Local) {
	w.i32(0); // sentinel tag — a `$dnode` never reaches tag-inspecting code
	w.ref_null(VA); // kids = null marks this as a leaf
	w.local_get(ents);
	w.local_get(leafhash);
	w.struct_new(types::T_DNODE);
}

/// Push a fresh branch `$dnode { tag, kids, ents: null, leafhash: 0 }`.
fn make_branch(w: &mut Wat, kids: Local) {
	w.i32(0);
	w.local_get(kids);
	w.ref_null(VA); // ents = null marks this as a branch
	w.i64(0);
	w.struct_new(types::T_DNODE);
}

/// Push a `$tuple(a, b, c)` (a 3-element entry: key, value, seq).
fn tuple3(w: &mut Wat, a: Local, b: Local, c: Local) {
	w.i32(types::TAG_TUPLE);
	w.local_get(a);
	w.local_get(b);
	w.local_get(c);
	w.array_new_fixed(VA, 3);
	w.struct_new(types::T_TUPLE);
}

/// Push a `$tuple(a, b)` (a 2-element `(key, value)` entry).
fn tuple2(w: &mut Wat, a: Local, b: Local) {
	w.i32(types::TAG_TUPLE);
	w.local_get(a);
	w.local_get(b);
	w.array_new_fixed(VA, 2);
	w.struct_new(types::T_TUPLE);
}

/// Push `t.elems[field]` for a `$tuple` reference held in local `t`.
fn tuple_elem(w: &mut Wat, t: Local, field: i32) {
	w.local_get(t)
		.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1);
	w.i32(field).array_get(VA);
}

/// Push `ents[i].elems[field]` — `field` of the `$tuple` entry at index `i`.
fn entry_elem(w: &mut Wat, ents: Local, i: Local, field: i32) {
	w.local_get(ents).local_get(i).array_get(VA);
	w.ref_cast(types::T_TUPLE).struct_get(types::T_TUPLE, 1);
	w.i32(field).array_get(VA);
}

/// Push an empty `$list { tag, elems: [], length: 0 }`.
fn empty_list(w: &mut Wat) {
	w.i32(types::TAG_LIST);
	w.i32(0).array_new_default(VA);
	w.i32(0);
	w.struct_new(types::T_LIST);
}

/// Push a fresh empty `$dict { tag, root: null, next_seq: 0 }`.
fn empty_dict(w: &mut Wat) {
	w.i32(types::TAG_DICT);
	w.ref_null(types::T_VALUE);
	w.i32(0);
	w.struct_new(types::T_DICT);
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
// Trie insert (recursive, path-copying).
// ---------------------------------------------------------------------------

/// Build `__dict_node_insert(node, hash, key, val, seq) -> $dnode`. `hash`/`seq`
/// arrive boxed (`$int`). Returns a path-copied root for the subtree. `self_idx`
/// is its own index (descent); `eq_idx` is `__eq` (bucket key match).
pub(crate) fn build_dict_node_insert_fn(self_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(5);
	let (node, hashb, key, val, seqb) = (w.param(0), w.param(1), w.param(2), w.param(3), w.param(4));
	let h = w.local(ValType::I64);
	let nd = w.local(types::dnode_ref_null());
	let kids = w.local(types::valarray_ref_null());
	let ents = w.local(types::valarray_ref_null());
	let lh = w.local(ValType::I64);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let found = w.local(ValType::I32);
	// Nullable so they're definitely-assigned (default null): the validator does
	// not merge assignments made only inside if/else arms. Always set before use.
	let newents = w.local(types::valarray_ref_null());
	let newkids = w.local(types::valarray_ref_null());
	let nib = w.local(ValType::I32);
	let oldnib = w.local(ValType::I32);
	let hs = w.local(ValType::I64); // hash >>> 4 (the child's remaining hash)
	let oldseq = w.local(types::value_ref());

	// h = unbox(hash).
	w.local_get(hashb)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_set(h);

	// Empty slot → a fresh single-entry leaf keyed on the remaining hash `h`.
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		tuple3(w, key, val, seqb);
		w.array_new_fixed(VA, 1).local_set(newents);
		make_leaf(w, newents, h);
		w.ret();
	});

	// nd = node; kids = nd.kids (null ⇒ leaf).
	w.local_get(node).ref_cast(types::T_DNODE).local_set(nd);
	w.local_get(nd)
		.struct_get(types::T_DNODE, 1)
		.local_set(kids);
	w.local_get(kids).ref_is_null();
	w.if_else(
		|w| {
			// --- leaf ---
			w.local_get(nd)
				.struct_get(types::T_DNODE, 2)
				.local_set(ents);
			w.local_get(nd).struct_get(types::T_DNODE, 3).local_set(lh);
			w.local_get(lh).local_get(h).i64_eq();
			w.if_else(
				|w| {
					// Same hash → replace (key present) or append into this bucket.
					w.local_get(ents).array_len().local_set(n);
					w.i32(-1).local_set(found);
					w.i32(0).local_set(i);
					w.block("fbrk", |w| {
						w.loop_("fscan", |w| {
							w.local_get(i).local_get(n).i32_ge_s().br_if("fbrk");
							entry_elem(w, ents, i, 0);
							w.local_get(key).call(eq_idx);
							w.if_(|w| {
								w.local_get(i).local_set(found);
								w.br("fbrk");
							});
							w.local_get(i).i32(1).i32_add().local_set(i);
							w.br("fscan");
						});
					});
					w.local_get(found).i32(0).i32_ge_s();
					w.if_else(
						|w| {
							// Replace value, keep the original seq.
							w.local_get(n).array_new_default(VA).local_set(newents);
							w.copy_loop(VA, newents, None, ents, None, n);
							entry_elem(w, ents, found, 2);
							w.local_set(oldseq);
							w.local_get(newents).local_get(found);
							tuple3(w, key, val, oldseq);
							w.array_set(VA);
						},
						|w| {
							// Append a new (key, val, seq) entry.
							w.local_get(n)
								.i32(1)
								.i32_add()
								.array_new_default(VA)
								.local_set(newents);
							w.copy_loop(VA, newents, None, ents, None, n);
							w.local_get(newents).local_get(n);
							tuple3(w, key, val, seqb);
							w.array_set(VA);
						},
					);
					make_leaf(w, newents, lh);
					w.ret();
				},
				|w| {
					// Different hash → split this leaf into a branch one level down.
					w.i32(FANOUT as i32)
						.array_new_default(VA)
						.local_set(newkids);
					// Place the existing leaf (bucket intact) at its nibble, hash >>> 4.
					w.local_get(lh)
						.i64(NIBBLE_MASK)
						.i64_and()
						.i32_wrap_i64()
						.local_set(oldnib);
					w.local_get(lh).i64(NIBBLE_BITS).i64_shr_u().local_set(hs);
					w.local_get(newkids).local_get(oldnib);
					make_leaf(w, ents, hs);
					w.array_set(VA);
					// Insert the new entry into the branch (recurses, splitting deeper
					// if it lands on the same nibble as the relocated leaf).
					w.local_get(h)
						.i64(NIBBLE_MASK)
						.i64_and()
						.i32_wrap_i64()
						.local_set(nib);
					w.local_get(h).i64(NIBBLE_BITS).i64_shr_u().local_set(hs);
					w.local_get(newkids).local_get(nib);
					w.local_get(newkids).local_get(nib).array_get(VA);
					w.i32(types::TAG_INT).local_get(hs).struct_new(types::T_INT);
					w.local_get(key)
						.local_get(val)
						.local_get(seqb)
						.call(self_idx);
					w.array_set(VA);
					make_branch(w, newkids);
					w.ret();
				},
			);
		},
		|w| {
			// --- branch --- copy the 16 child slots, recurse into the selected one.
			w.local_get(h)
				.i64(NIBBLE_MASK)
				.i64_and()
				.i32_wrap_i64()
				.local_set(nib);
			w.local_get(h).i64(NIBBLE_BITS).i64_shr_u().local_set(hs);
			w.i32(FANOUT as i32).local_set(n);
			w.i32(FANOUT as i32)
				.array_new_default(VA)
				.local_set(newkids);
			w.copy_loop(VA, newkids, None, kids, None, n);
			w.local_get(newkids).local_get(nib);
			w.local_get(kids).local_get(nib).array_get(VA);
			w.i32(types::TAG_INT).local_get(hs).struct_new(types::T_INT);
			w.local_get(key)
				.local_get(val)
				.local_get(seqb)
				.call(self_idx);
			w.array_set(VA);
			make_branch(w, newkids);
			w.ret();
		},
	);
	// Every arm of the outer `if_else` returns; this terminates the (divergent)
	// block so the `Empty` block type leaves a balanced stack.
	w.unreachable();
	w.finish()
}

// ---------------------------------------------------------------------------
// Trie structural equality (lets `__eq`'s dict case stay self-contained).
// ---------------------------------------------------------------------------

/// Build `__dict_node_eq(a, b) -> i32`. Two dicts with the same key set have the
/// same trie shape (splits are hash-determined, insertion-order-independent), so
/// this compares structure + per-leaf buckets, order-independently and ignoring
/// `seq`, via `__eq` (`eq_idx`). Either node may be null (an empty subtree).
pub(crate) fn build_dict_node_eq_fn(self_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let aents = w.local(types::valarray_ref_null());
	let bents = w.local(types::valarray_ref_null());
	let akids = w.local(types::valarray_ref_null());
	let bkids = w.local(types::valarray_ref_null());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let j = w.local(ValType::I32);
	let found = w.local(ValType::I32);

	// Null handling: both null → equal; exactly one null → unequal.
	w.local_get(a).ref_is_null();
	w.local_get(b).ref_is_null();
	w.i32_and();
	w.if_(|w| {
		w.i32(1).ret();
	});
	w.local_get(a).ref_is_null();
	w.local_get(b).ref_is_null();
	w.i32_or();
	w.if_(|w| {
		w.i32(0).ret();
	});

	// akids/bkids: null ⇒ leaf. A leaf-vs-branch mismatch ⇒ unequal.
	w.local_get(a)
		.ref_cast(types::T_DNODE)
		.struct_get(types::T_DNODE, 1)
		.local_set(akids);
	w.local_get(b)
		.ref_cast(types::T_DNODE)
		.struct_get(types::T_DNODE, 1)
		.local_set(bkids);
	w.local_get(akids).ref_is_null();
	w.local_get(bkids).ref_is_null();
	w.i32_ne();
	w.if_(|w| {
		w.i32(0).ret();
	});

	w.local_get(akids).ref_is_null();
	w.if_else(
		|w| {
			// --- both leaves --- equal-size buckets, every a-entry matched in b.
			w.local_get(a)
				.ref_cast(types::T_DNODE)
				.struct_get(types::T_DNODE, 2)
				.local_set(aents);
			w.local_get(b)
				.ref_cast(types::T_DNODE)
				.struct_get(types::T_DNODE, 2)
				.local_set(bents);
			w.local_get(aents).array_len().local_set(n);
			w.local_get(bents).array_len().local_get(n).i32_ne();
			w.if_(|w| {
				w.i32(0).ret();
			});
			w.i32(0).local_set(i);
			w.block("obrk", |w| {
				w.loop_("olp", |w| {
					w.local_get(i).local_get(n).i32_ge_s().br_if("obrk");
					w.i32(0).local_set(j);
					w.i32(0).local_set(found);
					w.block("ibrk", |w| {
						w.loop_("ilp", |w| {
							w.local_get(j).local_get(n).i32_ge_s().br_if("ibrk");
							// keys match? then values must match, else fail outright.
							entry_elem(w, aents, i, 0);
							entry_elem(w, bents, j, 0);
							w.call(eq_idx);
							w.if_(|w| {
								entry_elem(w, aents, i, 1);
								entry_elem(w, bents, j, 1);
								w.call(eq_idx).i32_eqz();
								w.if_(|w| {
									w.i32(0).ret();
								});
								w.i32(1).local_set(found);
								w.br("ibrk");
							});
							w.local_get(j).i32(1).i32_add().local_set(j);
							w.br("ilp");
						});
					});
					w.local_get(found).i32_eqz();
					w.if_(|w| {
						w.i32(0).ret();
					});
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("olp");
				});
			});
			w.i32(1).ret();
		},
		|w| {
			// --- both branches --- compare all 16 child slots pairwise.
			w.i32(0).local_set(i);
			w.block("cbrk", |w| {
				w.loop_("clp", |w| {
					w.local_get(i).i32(FANOUT as i32).i32_ge_s().br_if("cbrk");
					w.local_get(akids).local_get(i).array_get(VA);
					w.local_get(bkids).local_get(i).array_get(VA);
					w.call(self_idx).i32_eqz();
					w.if_(|w| {
						w.i32(0).ret();
					});
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("clp");
				});
			});
			w.i32(1).ret();
		},
	);
	// Both arms return; terminate the divergent body for the `(result i32)` type.
	w.unreachable();
	w.finish()
}

// ---------------------------------------------------------------------------
// Trie traversal: collect entries, count entries.
// ---------------------------------------------------------------------------

/// Build `__dict_collect(node, list) -> list`: append every entry of the subtree
/// (each the stored `$tuple(key, value, seq)`) to `list` via `__list_push`
/// (`push_idx`), returning the (in-place-grown) list. Order is arbitrary — the
/// caller sorts by `seq`. `self_idx` recurses into branch children.
pub(crate) fn build_dict_collect_fn(self_idx: u32, push_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (node, list) = (w.param(0), w.param(1));
	let kids = w.local(types::valarray_ref_null());
	let ents = w.local(types::valarray_ref_null());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// Null subtree → list unchanged.
	w.local_get(node).ref_is_null();
	w.if_(|w| {
		w.local_get(list).ret();
	});
	w.local_get(node)
		.ref_cast(types::T_DNODE)
		.struct_get(types::T_DNODE, 1)
		.local_set(kids);
	w.local_get(kids).ref_is_null();
	w.if_else(
		|w| {
			// leaf: push each entry tuple.
			w.local_get(node)
				.ref_cast(types::T_DNODE)
				.struct_get(types::T_DNODE, 2)
				.local_set(ents);
			w.local_get(ents).array_len().local_set(n);
			w.i32(0).local_set(i);
			w.block("brk", |w| {
				w.loop_("lp", |w| {
					w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
					w.local_get(list);
					w.local_get(ents).local_get(i).array_get(VA);
					w.call(push_idx).drop(); // push returns nothing; list grows in place
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("lp");
				});
			});
		},
		|w| {
			// branch: recurse into each child (collect tolerates a null child).
			w.i32(0).local_set(i);
			w.block("brk", |w| {
				w.loop_("lp", |w| {
					w.local_get(i).i32(FANOUT as i32).i32_ge_s().br_if("brk");
					w.local_get(kids).local_get(i).array_get(VA);
					w.local_get(list);
					w.call(self_idx).drop();
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("lp");
				});
			});
		},
	);
	w.local_get(list);
	w.finish()
}

/// Build `__dict_count(node) -> $int`: total entries in the subtree. Recursive.
pub(crate) fn build_dict_count_fn(self_idx: u32) -> Function {
	let mut w = Wat::new(1);
	let node = w.param(0);
	let kids = w.local(types::valarray_ref_null());
	let acc = w.local(ValType::I64);
	let i = w.local(ValType::I32);

	w.local_get(node).ref_is_null();
	w.if_(|w| {
		w.i32(types::TAG_INT).i64(0).struct_new(types::T_INT).ret();
	});
	w.local_get(node)
		.ref_cast(types::T_DNODE)
		.struct_get(types::T_DNODE, 1)
		.local_set(kids);
	w.local_get(kids).ref_is_null();
	w.if_(|w| {
		// leaf: bucket size.
		w.i32(types::TAG_INT);
		w.local_get(node)
			.ref_cast(types::T_DNODE)
			.struct_get(types::T_DNODE, 2)
			.array_len();
		w.i64_extend_i32_u();
		w.struct_new(types::T_INT).ret();
	});
	// branch: sum the children.
	w.i64(0).local_set(acc);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).i32(FANOUT as i32).i32_ge_s().br_if("brk");
			w.local_get(acc);
			w.local_get(kids).local_get(i).array_get(VA);
			w.call(self_idx)
				.ref_cast(types::T_INT)
				.struct_get(types::T_INT, 1);
			w.i64_add().local_set(acc);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_INT)
		.local_get(acc)
		.struct_new(types::T_INT);
	w.finish()
}

// ---------------------------------------------------------------------------
// Public dict builtins.
// ---------------------------------------------------------------------------

/// Build `__dict_insert(dict, key, value) -> dict`: hash the key, path-copy the
/// trie, bump `next_seq`. `hash_idx` = `__hash`, `node_idx` = `__dict_node_insert`.
pub(crate) fn build_dict_insert_fn(hash_idx: u32, node_idx: u32) -> Function {
	let mut w = Wat::new(3);
	let (dict, key, val) = (w.param(0), w.param(1), w.param(2));
	let ns = w.local(ValType::I32);
	let root = w.local(types::value_ref());
	let newroot = w.local(types::value_ref());

	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 2)
		.local_set(ns);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 1)
		.local_set(root);
	// newroot = node_insert(root, box(ns) as hash, key, val, box(ns) as seq).
	w.local_get(root);
	w.local_get(key).call(hash_idx); // hash = __hash(key)
	w.local_get(key);
	w.local_get(val);
	w.local_get(ns).i64_extend_i32_u().box_int(); // seq box (i31 when small)
	w.call(node_idx).local_set(newroot);
	// dict { tag, newroot, ns + 1 }.
	w.i32(types::TAG_DICT);
	w.local_get(newroot);
	w.local_get(ns).i32(1).i32_add();
	w.struct_new(types::T_DICT);
	w.finish()
}

/// Build `__dict_lookup(dict, key) -> option value`: descend the trie by hash
/// nibbles, then `__eq`-scan the leaf bucket. `hash_idx` = `__hash`, `eq_idx` =
/// `__eq`, `opt` builds the `some`/`none` result.
pub(crate) fn build_dict_lookup_fn(hash_idx: u32, eq_idx: u32, opt: OptionLits) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let h = w.local(ValType::I64);
	let node = w.local(types::value_ref());
	let kids = w.local(types::valarray_ref_null());
	let ents = w.local(types::valarray_ref_null());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let nib = w.local(ValType::I32);

	// Push a fresh `$str` for an interned data-segment literal (the variant name).
	let str_lit = |w: &mut Wat, (off, len): (u32, u32)| {
		w.i32(types::TAG_STR);
		w.i32(off as i32);
		w.i32(len as i32);
		w.array_new_data(types::T_BYTES, 0);
		w.struct_new(types::T_STR);
	};
	let none = |w: &mut Wat| {
		w.i32(types::TAG_VARIANT).i32(opt.none_tag as i32);
		str_lit(w, opt.none_name);
		w.array_new_fixed(VA, 0).struct_new(types::T_VARIANT);
	};

	w.local_get(key)
		.call(hash_idx)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_set(h);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 1)
		.local_set(node);
	w.loop_("desc", |w| {
		// Empty slot → none.
		w.local_get(node).ref_is_null();
		w.if_(|w| {
			none(w);
			w.ret();
		});
		w.local_get(node)
			.ref_cast(types::T_DNODE)
			.struct_get(types::T_DNODE, 1)
			.local_set(kids);
		w.local_get(kids).ref_is_null();
		w.if_else(
			|w| {
				// leaf: scan the bucket by `__eq`.
				w.local_get(node)
					.ref_cast(types::T_DNODE)
					.struct_get(types::T_DNODE, 2)
					.local_set(ents);
				w.local_get(ents).array_len().local_set(n);
				w.i32(0).local_set(i);
				w.block("sbrk", |w| {
					w.loop_("slp", |w| {
						w.local_get(i).local_get(n).i32_ge_s().br_if("sbrk");
						entry_elem(w, ents, i, 0);
						w.local_get(key).call(eq_idx);
						w.if_(|w| {
							// some(value).
							w.i32(types::TAG_VARIANT).i32(opt.some_tag as i32);
							str_lit(w, opt.some_name);
							entry_elem(w, ents, i, 1);
							w.array_new_fixed(VA, 1).struct_new(types::T_VARIANT);
							w.ret();
						});
						w.local_get(i).i32(1).i32_add().local_set(i);
						w.br("slp");
					});
				});
				none(w);
				w.ret();
			},
			|w| {
				// branch: descend one nibble.
				w.local_get(h)
					.i64(NIBBLE_MASK)
					.i64_and()
					.i32_wrap_i64()
					.local_set(nib);
				w.local_get(kids)
					.local_get(nib)
					.array_get(VA)
					.local_set(node);
				w.local_get(h).i64(NIBBLE_BITS).i64_shr_u().local_set(h);
			},
		);
		// Only the branch arm reaches here; continue the descent.
		w.br("desc");
	});
	// `loop` is divergent (every path rets); unreachable terminator for validation.
	w.unreachable();
	w.finish()
}

/// Build `__dict_size(dict) -> $int`: count `dict.root` via `__dict_count`.
pub(crate) fn build_dict_size_fn(count_idx: u32) -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 1);
	w.call(count_idx);
	w.finish()
}

/// Build `__dict_entries(dict) -> list`: collect the trie, then reorder by `seq`
/// into insertion order and strip `seq` to `$tuple(key, value)`. `collect_idx` =
/// `__dict_collect`. Placement is by `seq` index (O(n + next_seq)), no comparison
/// sort.
pub(crate) fn build_dict_entries_fn(collect_idx: u32) -> Function {
	let mut w = Wat::new(1);
	let dict = w.param(0);
	let lst = w.local(types::value_ref());
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let ns = w.local(ValType::I32);
	let tmp = w.local(types::valarray_ref());
	let out = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let wr = w.local(ValType::I32);
	let t = w.local(types::value_ref());
	let s = w.local(ValType::I32);
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());
	let slot = w.local(types::value_ref());

	// lst = collect(root, []).
	empty_list(&mut w);
	w.local_set(lst);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 1);
	w.local_get(lst).call(collect_idx).local_set(lst);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 2)
		.local_set(n);
	w.local_get(dict)
		.ref_cast(types::T_DICT)
		.struct_get(types::T_DICT, 2)
		.local_set(ns);

	// Scatter: tmp[seq] = (key, value), a sparse array indexed by insertion seq.
	w.local_get(ns).array_new_default(VA).local_set(tmp);
	w.i32(0).local_set(i);
	w.block("fbrk", |w| {
		w.loop_("flp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("fbrk");
			w.local_get(arr).local_get(i).array_get(VA).local_set(t);
			tuple_elem(w, t, 2);
			w.unbox_int().i32_wrap_i64().local_set(s);
			tuple_elem(w, t, 0);
			w.local_set(k);
			tuple_elem(w, t, 1);
			w.local_set(v);
			w.local_get(tmp).local_get(s);
			tuple2(w, k, v);
			w.array_set(VA);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("flp");
		});
	});

	// Gather: compact non-null slots (skipping seq gaps left by removes) into a
	// dense `out` of length n, in ascending-seq (insertion) order.
	w.local_get(n).array_new_default(VA).local_set(out);
	w.i32(0).local_set(wr);
	w.i32(0).local_set(i);
	w.block("gbrk", |w| {
		w.loop_("glp", |w| {
			w.local_get(i).local_get(ns).i32_ge_s().br_if("gbrk");
			w.local_get(tmp).local_get(i).array_get(VA).local_set(slot);
			w.local_get(slot).ref_is_null().i32_eqz();
			w.if_(|w| {
				w.local_get(out).local_get(wr).local_get(slot).array_set(VA);
				w.local_get(wr).i32(1).i32_add().local_set(wr);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("glp");
		});
	});

	w.i32(types::TAG_LIST)
		.local_get(out)
		.local_get(n)
		.struct_new(types::T_LIST);
	w.finish()
}

/// Build `__dict_remove(dict, key) -> dict`: rebuild from sorted entries, dropping
/// the matching key (so insertion order is preserved). `entries_idx`/`insert_idx`/
/// `eq_idx` = `__dict_entries`/`__dict_insert`/`__eq`.
pub(crate) fn build_dict_remove_fn(entries_idx: u32, insert_idx: u32, eq_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, key) = (w.param(0), w.param(1));
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let acc = w.local(types::value_ref());
	let t = w.local(types::value_ref());
	let k = w.local(types::value_ref());

	entries_list(&mut w, dict, entries_idx, arr, n);
	empty_dict(&mut w);
	w.local_set(acc);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(arr).local_get(i).array_get(VA).local_set(t);
			tuple_elem(w, t, 0);
			w.local_set(k);
			// Keep entries whose key ≠ the removed key.
			w.local_get(k).local_get(key).call(eq_idx).i32_eqz();
			w.if_(|w| {
				w.local_get(acc);
				w.local_get(k);
				tuple_elem(w, t, 1);
				w.call(insert_idx).local_set(acc);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(acc);
	w.finish()
}

/// Build `__dict_map(dict, f) -> dict`: rebuild applying `f` to each value (keys +
/// order preserved). `arity1` is `f`'s `(env, value)` indirect type.
pub(crate) fn build_dict_map_fn(entries_idx: u32, insert_idx: u32, arity1: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, f) = (w.param(0), w.param(1));
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let acc = w.local(types::value_ref());
	let t = w.local(types::value_ref());
	let k = w.local(types::value_ref());

	entries_list(&mut w, dict, entries_idx, arr, n);
	empty_dict(&mut w);
	w.local_set(acc);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(arr).local_get(i).array_get(VA).local_set(t);
			tuple_elem(w, t, 0);
			w.local_set(k);
			// acc = insert(acc, k, f(value)).
			w.local_get(acc);
			w.local_get(k);
			w.local_get(f).ref_cast(types::T_CLOSURE);
			tuple_elem(w, t, 1);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity1);
			w.call(insert_idx).local_set(acc);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(acc);
	w.finish()
}

/// Build `__dict_filter(dict, f) -> dict`: rebuild keeping entries where `f k v`
/// is true (order preserved). `arity2` is `f`'s `(env, key, value)` indirect type.
pub(crate) fn build_dict_filter_fn(entries_idx: u32, insert_idx: u32, arity2: u32) -> Function {
	let mut w = Wat::new(2);
	let (dict, f) = (w.param(0), w.param(1));
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let acc = w.local(types::value_ref());
	let t = w.local(types::value_ref());
	let k = w.local(types::value_ref());
	let v = w.local(types::value_ref());

	entries_list(&mut w, dict, entries_idx, arr, n);
	empty_dict(&mut w);
	w.local_set(acc);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(arr).local_get(i).array_get(VA).local_set(t);
			tuple_elem(w, t, 0);
			w.local_set(k);
			tuple_elem(w, t, 1);
			w.local_set(v);
			// keep = f(k, v) (unbox the `$bool`).
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(k).local_get(v);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity2);
			w.ref_cast(types::T_BOOL).struct_get(types::T_BOOL, 1);
			w.if_(|w| {
				w.local_get(acc)
					.local_get(k)
					.local_get(v)
					.call(insert_idx)
					.local_set(acc);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(acc);
	w.finish()
}

/// Shared preamble for remove/map/filter: `arr`/`n` = the backing array + length
/// of `__dict_entries(dict)` (sorted `(key, value)` tuples).
fn entries_list(w: &mut Wat, dict: Local, entries_idx: u32, arr: Local, n: Local) {
	let lst = w.local(types::value_ref());
	w.local_get(dict).call(entries_idx).local_set(lst);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 2)
		.local_set(n);
}
