// Module assembly + per-function body emission.
//
// `Module::build` lays out the wasm module: host imports for the builtins a
// program actually calls, one defined function per reachable IR function (dense
// `FuncId -> wasm-index` numbering after the imports), a passive data segment
// holding every string constant, and the entry export. `FnEmitter` walks one
// function's IR `Block`, mapping each `VarId` to a wasm local and each `Rvalue`
// to GC/numeric instructions.
//
// Uniform-boxed contract (see `lib.rs`): every IR function takes `n` boxed params
// and returns one boxed value, so signatures are arity-keyed; `var_reprs` says
// which locals are unboxed i64/f64/i32 and `Box`/`Unbox` mark the GC-ref
// boundaries the coercion pass already inserted.

use std::collections::{HashMap, HashSet};

use ir::{Atom, Block, Callee, Const, GlobalInit, IrProgram, PreEval, Repr, Rvalue, StmtKind};
use wasm_encoder::{
	CodeSection, ConstExpr, DataCountSection, DataSection, ElementSection, Elements, ExportKind,
	ExportSection, Function, FunctionSection, GlobalSection, GlobalType, HeapType, ImportSection,
	Instruction, Module as WasmModule, RefType, TableSection, TableType, TypeSection, ValType,
};

/// A reachable IR global realized as a lazily-initialized wasm value: a cached
/// value (`val_idx`) behind an `i32` init flag (`init_idx`), built on first
/// access. (Builtin globals are call-only; Const globals aren't realized yet.)
#[derive(Clone)]
struct GlobalSlot {
	val_idx: u32,
	init_idx: u32,
	kind: GlobalKind,
}

#[derive(Clone)]
enum GlobalKind {
	/// A top-level def: run its thunk (wasm index) once.
	Thunk(u32),
	/// A trait-instance method dict: build a `$methoddict` of builtin-wrapper
	/// closures (each method's wrapper wasm index).
	MethodDict(Vec<u32>),
}

/// Wasm indices of the synthetic runtime helpers, available to every function.
/// `None` = not emitted (the reachable program doesn't need it).
#[derive(Clone, Copy, Default)]
struct Runtime {
	/// `__eq(value, value) -> i32` — structural equality.
	eq_fn: Option<u32>,
	/// `__getfield(record, name) -> value` — record field access by name.
	getfield_fn: Option<u32>,
	/// `__record_update(record, name, value) -> record` — one override of a copy.
	record_update_fn: Option<u32>,
	/// `__list_tail(list, n) -> list` — the `...rest` tail of a list pattern.
	list_tail_fn: Option<u32>,
	/// `__arrconcat(a, b) -> valarray` — concatenate two value arrays (list spread).
	arrconcat_fn: Option<u32>,
	/// `__bytesconcat(a, b) -> bytes` — concatenate two byte arrays (`++` / interp).
	bytesconcat_fn: Option<u32>,
	/// `__tostring(value) -> str` — `vm::Value`'s `Display` in wasm (scalars only).
	tostring_fn: Option<u32>,
	/// `__int_str(i64) -> str` — decimal formatting, used by `__tostring`.
	int_str_fn: Option<u32>,
	/// `__list_build(n, f) -> list` — tabulate `[f 0, ..., f (n-1)]`.
	list_build_fn: Option<u32>,
	/// `__list_collect(n, f) -> list` — tabulate keeping only `f`'s `some` results.
	list_collect_fn: Option<u32>,
	/// `__bytes_build(n, f) -> bytes` — tabulate a byte sequence.
	bytes_build_fn: Option<u32>,
	/// `__dict_insert(dict, k, v) -> dict` — replace-or-append (linear scan via `__eq`).
	dict_insert_fn: Option<u32>,
	/// `__dict_lookup(dict, k) -> option v` — linear scan via `__eq`.
	dict_lookup_fn: Option<u32>,
	/// `__dict_remove(dict, k) -> dict` — drop the matching entry (linear scan).
	dict_remove_fn: Option<u32>,
	/// `__dict_map(dict, f) -> dict` — `f` over each value, keys preserved.
	dict_map_fn: Option<u32>,
	/// `__dict_filter(dict, f) -> dict` — keep entries where `f key value` is true.
	dict_filter_fn: Option<u32>,
	/// Host import `float_to_str(f64, $bytes buf) -> i32 len` — float formatting
	/// (delegated to the host, like a browser's `String(x)`), used by `__tostring`.
	float_to_str_fn: Option<u32>,
	/// Data-segment offsets/lengths for the literal strings `__tostring` needs.
	lits: ToStringLits,
	/// `some`/`none` variant info for `__dict_lookup` to build its `option` result.
	opt: OptionLits,
	/// `lt`/`eq`/`gt` variant info for the `*-compare` wrappers to build their
	/// `ordering` result.
	ord: OrderingLits,
	/// `__wire_fp(i64 hash, ref $value schema) -> i64` — recursive FNV-1a mix over
	/// a `wire-schema` value tree (the structural fingerprint).
	wire_fp_fn: Option<u32>,
	/// `__wire_mix_str(i64 hash, ref $value str) -> i64` — mix a string's byte
	/// length + bytes.
	wire_mix_str_fn: Option<u32>,
	/// `__wire_mix_len(i64 hash, i64 n) -> i64` — mix `n`'s 8 little-endian bytes.
	wire_mix_len_fn: Option<u32>,
	/// The `wire-schema` enum's per-variant tags, for the codec helpers' dispatch.
	wire: WireTags,
}

/// The within-enum tags of the `wire-schema` prelude enum's variants, resolved
/// from the enum table so the codec helpers can dispatch on a schema node's
/// runtime `vtag` rather than its name string.
#[derive(Clone, Copy, Default)]
struct WireTags {
	s_int: u32,
	s_float: u32,
	s_bool: u32,
	s_string: u32,
	s_bytes: u32,
	s_duration: u32,
	s_nothing: u32,
	s_list: u32,
	s_dict: u32,
	s_enum_ref: u32,
	s_tuple: u32,
	s_record: u32,
	s_enum: u32,
}

/// What an `*-compare` wrapper needs to construct an `ordering` `$variant`: each
/// variant's within-enum tag and its interned display-name string `(off, len)`.
#[derive(Clone, Copy, Default)]
struct OrderingLits {
	lt_tag: u32,
	eq_tag: u32,
	gt_tag: u32,
	lt_name: (u32, u32),
	eq_name: (u32, u32),
	gt_name: (u32, u32),
}

/// What `__dict_lookup` needs to construct `some v` / `none` `$variant`s: each
/// variant's within-enum tag and its interned display-name string `(off, len)`.
#[derive(Clone, Copy, Default)]
struct OptionLits {
	some_tag: u32,
	none_tag: u32,
	some_name: (u32, u32),
	none_name: (u32, u32),
}

/// `(offset, len)` of each fixed literal `__tostring` emits, in the shared data
/// segment.
#[derive(Clone, Copy, Default)]
struct ToStringLits {
	unit: (u32, u32),
	tru: (u32, u32),
	fals: (u32, u32),
	lparen: (u32, u32),
	rparen: (u32, u32),
	lbrack: (u32, u32),
	rbrack: (u32, u32),
	lbrace: (u32, u32),
	rbrace: (u32, u32),
	comma_sp: (u32, u32), // ", "
	colon_sp: (u32, u32), // ": "
	space: (u32, u32),    // " "
	ref_pfx: (u32, u32),  // "ref "
}

/// Which runtime helpers a reachable program needs. `eq` is forced on whenever
/// `getfield`/`record_update` is (both compare name strings via `__eq`).
#[derive(Default)]
struct Needs {
	eq: bool,
	getfield: bool,
	record_update: bool,
	list_tail: bool,
	arrconcat: bool,
	bytesconcat: bool,
	tostring: bool,
}

fn scan_needs(b: &Block, n: &mut Needs) {
	fn rv(rv: &Rvalue, n: &mut Needs) {
		match rv {
			Rvalue::Bin(ir::BinOp::Eq | ir::BinOp::Ne, _, _) => n.eq = true,
			Rvalue::GetField(..) => {
				n.getfield = true;
				n.eq = true;
			}
			Rvalue::RecordUpdate { .. } => {
				n.record_update = true;
				n.eq = true;
			}
			Rvalue::MakeList(items) => {
				if items.iter().any(|it| matches!(it, ir::ListItem::Spread(_))) {
					n.arrconcat = true;
				}
			}
			Rvalue::Bin(ir::BinOp::Concat, _, _) | Rvalue::Interpolate(_) => n.bytesconcat = true,
			_ => {}
		}
	}
	fn pat(p: &ir::Pattern, n: &mut Needs) {
		match p {
			ir::Pattern::List {
				rest: Some(ir::ListRest::Bind(_)),
				items,
			} => {
				n.list_tail = true;
				items.iter().for_each(|p| pat(p, n));
			}
			ir::Pattern::List { items, .. } => items.iter().for_each(|p| pat(p, n)),
			ir::Pattern::Variant { fields, .. } | ir::Pattern::Tuple(fields) => {
				fields.iter().for_each(|p| pat(p, n))
			}
			ir::Pattern::Record { fields, .. } => {
				// Record patterns match fields via `__getfield` (which uses `__eq`).
				n.getfield = true;
				n.eq = true;
				fields.iter().for_each(|(_, p)| pat(p, n));
			}
			_ => {}
		}
	}
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, r) | StmtKind::Discard(r) => rv(r, n),
			StmtKind::If(_, t, e) => {
				scan_needs(t, n);
				scan_needs(e, n);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					scan_needs(b, n);
				}
				scan_needs(default, n);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					pat(&a.pattern, n);
					scan_needs(&a.body, n);
				}
			}
			StmtKind::Loop(b) => scan_needs(b, n),
			_ => {}
		}
	}
}

use crate::types::{self, FuncTypes};
use crate::{builtin_globals, Diagnostics, Reach};

/// A host primitive's calling shape: how many boxed args it takes, and whether it
/// returns a boxed value (vs. nothing — in which case the caller materializes the
/// Pluma `nothing` result).
struct HostSig {
	arity: usize,
	returns_value: bool,
}

/// The host signature for a builtin tag, or `None` if this backend doesn't yet
/// import it. Grows with milestone coverage (M7 brings the rest).
fn host_sig(tag: &str) -> Option<HostSig> {
	match tag {
		"print" => Some(HostSig {
			arity: 1,
			returns_value: false,
		}),
		_ => None,
	}
}

/// Pure-compute builtins emitted inline at the call site (no host import, no
/// synthetic helper). They operate on the GC `$value` layout directly. Grows as
/// more of the builtin surface moves to native WasmGC.
fn is_inline_builtin(tag: &str) -> bool {
	matches!(
		tag,
		"list-get"
			| "list-length"
			| "bytes-get"
			| "bytes-length"
			| "bytes-as-string"
			| "string-to-bytes"
			// the in-place list mutation: `array.set` on the `$valarray`.
			| "list-set"
			// mutable cells: a `$ref` struct with a mutable boxed-value field.
			// `ref-update` reads, applies a closure, writes back (closure call inline).
			| "ref-new"
			| "ref-get"
			| "ref-set"
			| "ref-update"
			// dicts: the trivial accessors over the `$dict` entries array. The
			// rebuild/scan/closure ops (insert/lookup/remove/map/filter) are helpers.
			| "dict-empty"
			| "dict-size"
			| "dict-entries"
			// math primitives WasmGC does with one f64/i64 opcode (the
			// transcendentals log/exp/sin/cos still need a host import).
			| "math-sqrt"
			| "math-to-int"
			| "math-to-float"
	)
}

/// Transcendental float math with no WasmGC opcode (log/log10/log2/exp/sin/cos).
/// Each is a `(f64) -> f64` host import (the same libm call the VM makes); the
/// `$float` box/unbox is emitted in wasm around the call.
fn is_f64_unary_host(tag: &str) -> bool {
	matches!(
		tag,
		"math-log" | "math-log10" | "math-log2" | "math-exp" | "math-sin" | "math-cos"
	)
}

pub(crate) struct Module;

impl Module {
	pub fn build(p: &IrProgram, reach: &Reach, diags: &mut Diagnostics) -> Vec<u8> {
		let builtin_g = builtin_globals(p);

		// Host imports: the builtin tags actually called in reachable functions.
		// `to-string` is special — it's implemented in wasm (`__tostring`), not
		// imported — so route it to a flag rather than the import table.
		let mut host_index: HashMap<String, u32> = HashMap::new();
		let mut host_order: Vec<String> = Vec::new();
		let mut tostring_called = false;
		let mut list_build_called = false;
		let mut list_collect_called = false;
		let mut bytes_build_called = false;
		let mut bytes_concat_called = false;
		let mut dict_insert_called = false;
		let mut dict_lookup_called = false;
		let mut dict_remove_called = false;
		let mut dict_map_called = false;
		let mut dict_filter_called = false;
		let mut wire_fingerprint_called = false;
		for &fid in &reach.order {
			collect_host_calls(&p.functions[fid as usize].body, &builtin_g, |tag| {
				if tag == "to-string" {
					tostring_called = true;
					return;
				}
				// Higher-order builders implemented as synthetic wasm helpers
				// (loop + closure call), not host imports.
				if tag == "list-build" {
					list_build_called = true;
					return;
				}
				if tag == "list-collect" {
					list_collect_called = true;
					return;
				}
				if tag == "bytes-build" {
					bytes_build_called = true;
					return;
				}
				// bytes.concat reuses the `__bytesconcat` helper inline.
				if tag == "bytes-concat" {
					bytes_concat_called = true;
					return;
				}
				// dict scan/rebuild/closure ops: synthetic helpers (the trivial
				// accessors empty/size/entries go through `is_inline_builtin`).
				match tag {
					"dict-insert" => {
						dict_insert_called = true;
						return;
					}
					"dict-lookup" => {
						dict_lookup_called = true;
						return;
					}
					"dict-remove" => {
						dict_remove_called = true;
						return;
					}
					"dict-map" => {
						dict_map_called = true;
						return;
					}
					"dict-filter" => {
						dict_filter_called = true;
						return;
					}
					// `wire-fingerprint` walks the schema value tree (synthetic helper).
					"wire-fingerprint" => {
						wire_fingerprint_called = true;
						return;
					}
					_ => {}
				}
				// Pure-compute builtins emitted inline at the call site (no import).
				if is_inline_builtin(tag) {
					return;
				}
				// Unary float math: a `(f64) -> f64` host import (box/unbox emitted in
				// wasm), registered like any import but typed separately below.
				if is_f64_unary_host(tag) {
					if !host_index.contains_key(tag) {
						host_index.insert(tag.to_string(), host_order.len() as u32);
						host_order.push(tag.to_string());
					}
					return;
				}
				if !host_index.contains_key(tag) {
					if host_sig(tag).is_none() {
						diags.push(format!("unsupported host builtin `{tag}`"));
						return;
					}
					host_index.insert(tag.to_string(), host_order.len() as u32);
					host_order.push(tag.to_string());
				}
			});
		}
		// `__tostring` delegates float formatting to a host import.
		if tostring_called {
			host_index.insert("float_to_str".to_string(), host_order.len() as u32);
			host_order.push("float_to_str".to_string());
		}
		let num_imports = host_order.len() as u32;

		// Dense FuncId -> wasm function index (imports occupy the low indices).
		let mut wasm_index: HashMap<u32, u32> = HashMap::new();
		for (i, &fid) in reach.order.iter().enumerate() {
			wasm_index.insert(fid, num_imports + i as u32);
		}

		// `fun { body }` lowers to a function with *zero* IR params, but its type is
		// `nothing -> a` (arity 1) — its call sites pass the `()` arg. The bytecode
		// VM tolerates the arity mismatch; `call_indirect` does not, so give every
		// such closure a phantom param (wasm arity 1) to match its callers. These are
		// exactly the MakeClosure'd functions with no IR params.
		let mut zero_arg_closures: HashSet<u32> = HashSet::new();
		for &fid in &reach.order {
			collect_zero_arg_closures(&p.functions[fid as usize].body, p, &mut zero_arg_closures);
		}
		let wasm_arity = |fid: u32, params: usize| -> usize {
			params
				+ if zero_arg_closures.contains(&fid) {
					1
				} else {
					0
				}
		};

		// Synthetic runtime helpers occupy wasm indices right after the IR
		// functions: the `__*` helpers (only those the program needs), then one
		// wrapper per pure-compute builtin used as a first-class value (a method-dict
		// method). Indices must be fixed up-front so emission can reference them.
		let n_ir = reach.order.len() as u32;
		let synth_base = num_imports + n_ir;
		let mut needs = Needs::default();
		for &fid in &reach.order {
			scan_needs(&p.functions[fid as usize].body, &mut needs);
		}
		needs.tostring |= tostring_called;
		// `__tostring` formats compounds structurally (folding byte arrays with
		// `__bytesconcat`) and formats its INT case via `__int_str`. `bytes.concat`
		// also reuses `__bytesconcat`.
		needs.bytesconcat |= bytes_concat_called;
		if needs.tostring {
			needs.bytesconcat = true;
		}
		// dict insert/lookup/remove compare keys with `__eq`.
		needs.eq |= dict_insert_called || dict_lookup_called || dict_remove_called;
		// Helper indices, assigned in a fixed order; `next` walks past each present one.
		let mut next_synth = synth_base;
		let mut take = |present: bool| -> Option<u32> {
			present.then(|| {
				let i = next_synth;
				next_synth += 1;
				i
			})
		};
		let mut runtime = Runtime {
			eq_fn: take(needs.eq),
			getfield_fn: take(needs.getfield),
			record_update_fn: take(needs.record_update),
			list_tail_fn: take(needs.list_tail),
			arrconcat_fn: take(needs.arrconcat),
			bytesconcat_fn: take(needs.bytesconcat),
			tostring_fn: take(needs.tostring),
			int_str_fn: take(needs.tostring),
			list_build_fn: take(list_build_called),
			list_collect_fn: take(list_collect_called),
			bytes_build_fn: take(bytes_build_called),
			dict_insert_fn: take(dict_insert_called),
			dict_lookup_fn: take(dict_lookup_called),
			dict_remove_fn: take(dict_remove_called),
			dict_map_fn: take(dict_map_called),
			dict_filter_fn: take(dict_filter_called),
			float_to_str_fn: host_index.get("float_to_str").copied(),
			lits: ToStringLits::default(),
			opt: OptionLits::default(),
			ord: OrderingLits::default(),
			// `wire-fingerprint` needs all three helpers (fp recurses, fp+mix_str
			// both mix lengths). Allocate them together so the set is all-or-nothing.
			wire_fp_fn: take(wire_fingerprint_called),
			wire_mix_str_fn: take(wire_fingerprint_called),
			wire_mix_len_fn: take(wire_fingerprint_called),
			wire: WireTags::default(),
		};
		let wrapper_base = next_synth;

		let mut sorted_globals: Vec<u32> = reach.globals.iter().copied().collect();
		sorted_globals.sort_unstable();

		// Reachable method-dict globals whose methods are all wrappable builtins;
		// collect the distinct wrapper tags (assigned indices in first-seen order).
		let mut wrapper_idx: HashMap<String, u32> = HashMap::new();
		let mut wrapper_order: Vec<String> = Vec::new();
		let mut methoddicts: Vec<(u32, Vec<String>)> = Vec::new();
		for &gid in &sorted_globals {
			if let GlobalInit::PreEvaluated(PreEval::MethodDict(ms)) = &p.globals[gid as usize] {
				let mut tags = Vec::new();
				let mut ok = true;
				for m in ms {
					match m {
						PreEval::Builtin(t) if builtin_arity(t).is_some() => tags.push(t.clone()),
						_ => {
							ok = false;
							break;
						}
					}
				}
				if !ok {
					diags.push(format!(
						"method-dict global {gid} has an unsupported method"
					));
					continue;
				}
				for t in &tags {
					if !wrapper_idx.contains_key(t) {
						wrapper_idx.insert(t.clone(), wrapper_base + wrapper_order.len() as u32);
						wrapper_order.push(t.clone());
					}
				}
				methoddicts.push((gid, tags));
			}
		}

		// Lazily-initialized globals: two wasm globals each (cached value + init
		// flag). Top-level-def thunks and method-dicts; builtins are call-only and
		// Const globals aren't realized yet.
		let mut gmap: HashMap<u32, GlobalSlot> = HashMap::new();
		let mut globals_sec = GlobalSection::new();
		let mut gidx = 0u32;
		let alloc_slot = |globals_sec: &mut GlobalSection, gidx: &mut u32| {
			let val_idx = *gidx;
			globals_sec.global(
				GlobalType {
					val_type: types::value_ref(),
					mutable: true,
					shared: false,
				},
				&ConstExpr::ref_null(HeapType::Concrete(types::T_VALUE)),
			);
			globals_sec.global(
				GlobalType {
					val_type: ValType::I32,
					mutable: true,
					shared: false,
				},
				&ConstExpr::i32_const(0),
			);
			*gidx += 2;
			(val_idx, val_idx + 1)
		};
		for &gid in &sorted_globals {
			let kind = match &p.globals[gid as usize] {
				GlobalInit::Thunk(fid) => wasm_index.get(&fid.0).map(|&w| GlobalKind::Thunk(w)),
				_ => None,
			};
			if let Some(kind) = kind {
				let (val_idx, init_idx) = alloc_slot(&mut globals_sec, &mut gidx);
				gmap.insert(
					gid,
					GlobalSlot {
						val_idx,
						init_idx,
						kind,
					},
				);
			}
		}
		for (gid, tags) in &methoddicts {
			let wrappers: Vec<u32> = tags.iter().map(|t| wrapper_idx[t]).collect();
			let (val_idx, init_idx) = alloc_slot(&mut globals_sec, &mut gidx);
			gmap.insert(
				*gid,
				GlobalSlot {
					val_idx,
					init_idx,
					kind: GlobalKind::MethodDict(wrappers),
				},
			);
		}

		// String-constant pool: one passive data segment, every `Const::Str`
		// concatenated, recorded by (offset, len).
		let mut strpool = StrPool::default();
		for &fid in &reach.order {
			scan_strings(&p.functions[fid as usize].body, &mut strpool, &p.enums);
		}
		// `__tostring`'s fixed literals go in the same data segment.
		if needs.tostring {
			runtime.lits = ToStringLits {
				unit: strpool.intern("()"),
				tru: strpool.intern("true"),
				fals: strpool.intern("false"),
				lparen: strpool.intern("("),
				rparen: strpool.intern(")"),
				lbrack: strpool.intern("["),
				rbrack: strpool.intern("]"),
				lbrace: strpool.intern("{"),
				rbrace: strpool.intern("}"),
				comma_sp: strpool.intern(", "),
				colon_sp: strpool.intern(": "),
				space: strpool.intern(" "),
				ref_pfx: strpool.intern("ref "),
			};
		}
		// `__dict_lookup` builds `some v` / `none`; intern those variant display
		// names and resolve their within-enum tags (the `option` enum).
		if runtime.dict_lookup_fn.is_some() {
			let opt_enum = p
				.enums
				.iter()
				.find(|(_, vs)| vs.iter().any(|(n, _)| n == "some"))
				.map(|(name, _)| name.clone());
			match (
				opt_enum,
				variant_tag_in(&p.enums, "some"),
				variant_tag_in(&p.enums, "none"),
			) {
				(Some(en), Some(some_tag), Some(none_tag)) => {
					runtime.opt = OptionLits {
						some_tag,
						none_tag,
						some_name: strpool.intern(&variant_display(&en, some_tag, &p.enums)),
						none_name: strpool.intern(&variant_display(&en, none_tag, &p.enums)),
					};
				}
				_ => diags.push("dict.lookup needs the `option` enum".to_string()),
			}
		}
		// The `*-compare` wrappers build an `ordering` variant; intern its `lt`/
		// `eq`/`gt` display names and resolve their within-enum tags.
		if wrapper_order.iter().any(|t| t.ends_with("-compare")) {
			let ord_enum = p
				.enums
				.iter()
				.find(|(_, vs)| vs.iter().any(|(n, _)| n == "lt"))
				.map(|(name, _)| name.clone());
			match (
				ord_enum,
				variant_tag_in(&p.enums, "lt"),
				variant_tag_in(&p.enums, "eq"),
				variant_tag_in(&p.enums, "gt"),
			) {
				(Some(en), Some(lt_tag), Some(eq_tag), Some(gt_tag)) => {
					runtime.ord = OrderingLits {
						lt_tag,
						eq_tag,
						gt_tag,
						lt_name: strpool.intern(&variant_display(&en, lt_tag, &p.enums)),
						eq_name: strpool.intern(&variant_display(&en, eq_tag, &p.enums)),
						gt_name: strpool.intern(&variant_display(&en, gt_tag, &p.enums)),
					};
				}
				_ => diags.push("`compare` needs the `ordering` enum".to_string()),
			}
		}
		// The `wire` codec helpers dispatch on a schema node's `vtag`; resolve the
		// `wire-schema` enum's per-variant tags (declaration order = wire tag).
		if runtime.wire_fp_fn.is_some() {
			match p.enums.get("__prelude__.wire-schema") {
				Some(vs) => {
					let pos = |name: &str| vs.iter().position(|(n, _)| n == name).map(|i| i as u32);
					match (
						pos("s-int"),
						pos("s-float"),
						pos("s-bool"),
						pos("s-string"),
						pos("s-bytes"),
						pos("s-duration"),
						pos("s-nothing"),
						pos("s-list"),
						pos("s-dict"),
						pos("s-enum-ref"),
						pos("s-tuple"),
						pos("s-record"),
						pos("s-enum"),
					) {
						(
							Some(s_int),
							Some(s_float),
							Some(s_bool),
							Some(s_string),
							Some(s_bytes),
							Some(s_duration),
							Some(s_nothing),
							Some(s_list),
							Some(s_dict),
							Some(s_enum_ref),
							Some(s_tuple),
							Some(s_record),
							Some(s_enum),
						) => {
							runtime.wire = WireTags {
								s_int,
								s_float,
								s_bool,
								s_string,
								s_bytes,
								s_duration,
								s_nothing,
								s_list,
								s_dict,
								s_enum_ref,
								s_tuple,
								s_record,
								s_enum,
							};
						}
						_ => diags.push("`wire` needs the `wire-schema` enum variants".to_string()),
					}
				}
				None => diags.push("`wire` needs the `wire-schema` enum".to_string()),
			}
		}

		// Function-type interning + section building.
		let mut ftypes = FuncTypes::new();

		let mut imports = ImportSection::new();
		for tag in &host_order {
			let ty = if tag == "float_to_str" {
				ftypes.for_float_to_str()
			} else if is_f64_unary_host(tag) {
				ftypes.for_f64_unary()
			} else {
				let sig = host_sig(tag).unwrap();
				ftypes.for_host(sig.arity, sig.returns_value)
			};
			imports.import("pluma", tag, wasm_encoder::EntityType::Function(ty));
		}

		let mut functions = FunctionSection::new();
		let mut code = CodeSection::new();
		for &fid in &reach.order {
			let f = &p.functions[fid as usize];
			let arity = wasm_arity(fid, f.params.len());
			let extra_params = (arity - f.params.len()) as u32;
			functions.function(ftypes.for_arity(arity));
			let mut em = FnEmitter::new(
				f,
				&wasm_index,
				&host_index,
				&builtin_g,
				&gmap,
				&runtime,
				&strpool,
				&p.enums,
				&mut ftypes,
				extra_params,
				diags,
			);
			let func = em.emit();
			code.function(&func);
		}
		// Append the synthetic helpers (after the IR functions, in the same fixed
		// order their indices were assigned), then the builtin wrappers.
		if let Some(idx) = runtime.eq_fn {
			functions.function(ftypes.for_eq());
			code.function(&build_eq_fn(idx));
		}
		if runtime.getfield_fn.is_some() {
			let eq = runtime.eq_fn.expect("getfield needs eq");
			functions.function(ftypes.for_helper(2));
			code.function(&build_getfield_fn(eq));
		}
		if runtime.record_update_fn.is_some() {
			let eq = runtime.eq_fn.expect("record_update needs eq");
			functions.function(ftypes.for_helper(3));
			code.function(&build_record_update_fn(eq));
		}
		if runtime.list_tail_fn.is_some() {
			functions.function(ftypes.for_helper(2));
			code.function(&build_list_tail_fn());
		}
		if runtime.arrconcat_fn.is_some() {
			functions.function(ftypes.for_arrconcat());
			code.function(&build_arrconcat_fn());
		}
		if runtime.bytesconcat_fn.is_some() {
			functions.function(ftypes.for_bytesconcat());
			code.function(&build_bytesconcat_fn());
		}
		if let Some(ts) = runtime.tostring_fn {
			let int_str = runtime.int_str_fn.expect("tostring needs int_str");
			let bc = runtime.bytesconcat_fn.expect("tostring needs bytesconcat");
			let f2s = runtime
				.float_to_str_fn
				.expect("tostring needs float_to_str");
			functions.function(ftypes.for_helper(1));
			code.function(&build_tostring_fn(ts, int_str, bc, f2s, runtime.lits));
		}
		if runtime.int_str_fn.is_some() {
			functions.function(ftypes.for_helper(1));
			code.function(&build_int_str_fn());
		}
		if runtime.list_build_fn.is_some() {
			let arity1 = ftypes.for_arity(1);
			functions.function(ftypes.for_helper(2));
			code.function(&build_list_build_fn(arity1));
		}
		if runtime.list_collect_fn.is_some() {
			let arity1 = ftypes.for_arity(1);
			functions.function(ftypes.for_helper(2));
			code.function(&build_list_collect_fn(arity1));
		}
		if runtime.bytes_build_fn.is_some() {
			let arity1 = ftypes.for_arity(1);
			functions.function(ftypes.for_helper(2));
			code.function(&build_bytes_build_fn(arity1));
		}
		if runtime.dict_insert_fn.is_some() {
			let eq = runtime.eq_fn.expect("dict_insert needs eq");
			functions.function(ftypes.for_helper(3));
			code.function(&build_dict_insert_fn(eq));
		}
		if runtime.dict_lookup_fn.is_some() {
			let eq = runtime.eq_fn.expect("dict_lookup needs eq");
			functions.function(ftypes.for_helper(2));
			code.function(&build_dict_lookup_fn(eq, runtime.opt));
		}
		if runtime.dict_remove_fn.is_some() {
			let eq = runtime.eq_fn.expect("dict_remove needs eq");
			functions.function(ftypes.for_helper(2));
			code.function(&build_dict_remove_fn(eq));
		}
		if runtime.dict_map_fn.is_some() {
			let arity1 = ftypes.for_arity(1);
			functions.function(ftypes.for_helper(2));
			code.function(&build_dict_map_fn(arity1));
		}
		if runtime.dict_filter_fn.is_some() {
			let arity2 = ftypes.for_arity(2);
			functions.function(ftypes.for_helper(2));
			code.function(&build_dict_filter_fn(arity2));
		}
		// `wire` codec helpers (allocated as a set; emit in the same order).
		if let (Some(fp), Some(mix_str), Some(mix_len)) = (
			runtime.wire_fp_fn,
			runtime.wire_mix_str_fn,
			runtime.wire_mix_len_fn,
		) {
			let mix_val_ty = ftypes.for_wire_mix_val();
			let mix_len_ty = ftypes.for_wire_mix_len();
			functions.function(mix_val_ty);
			code.function(&build_wire_fp_fn(fp, mix_str, mix_len, runtime.wire));
			functions.function(mix_val_ty);
			code.function(&build_wire_mix_str_fn(mix_len));
			functions.function(mix_len_ty);
			code.function(&build_wire_mix_len_fn());
		}
		for tag in &wrapper_order {
			let arity = builtin_arity(tag).unwrap();
			functions.function(ftypes.for_arity(arity));
			match build_builtin_wrapper(tag, &runtime.ord) {
				Some(f) => {
					code.function(&f);
				}
				None => {
					diags.push(format!("builtin wrapper `{tag}`"));
					code.function(&Function::new(vec![]));
				}
			}
		}

		// A funcref table holds every defined function at its wasm index, so
		// `CallClosure` can `call_indirect` through a closure's stored `fn_index`.
		let n_synth = (wrapper_base - synth_base) + wrapper_order.len() as u32;
		let total = num_imports + n_ir + n_synth;
		let mut tables = TableSection::new();
		tables.table(TableType {
			element_type: RefType::FUNCREF,
			table64: false,
			minimum: total as u64,
			maximum: Some(total as u64),
			shared: false,
		});
		let mut elements = ElementSection::new();
		let defined: Vec<u32> = (num_imports..total).collect();
		elements.active(
			Some(0),
			&ConstExpr::i32_const(num_imports as i32),
			Elements::Functions(defined.into()),
		);

		// Sections must be encoded in canonical order; `ftypes`/`strpool` are now
		// final, so the type section is built last but placed first.
		let types: TypeSection = ftypes.encode();

		let mut exports = ExportSection::new();
		if let Some(&w) = wasm_index.get(&p.entry.0) {
			exports.export("_entry", ExportKind::Func, w);
		}

		let mut data = DataSection::new();
		data.passive(strpool.bytes.iter().copied());
		let data_count = DataCountSection { count: 1 };

		let mut module = WasmModule::new();
		module.section(&types);
		module.section(&imports);
		module.section(&functions);
		module.section(&tables);
		module.section(&globals_sec);
		module.section(&exports);
		module.section(&elements);
		module.section(&data_count);
		module.section(&code);
		module.section(&data);
		module.finish()
	}
}

#[derive(Default)]
struct StrPool {
	bytes: Vec<u8>,
	at: HashMap<String, (u32, u32)>,
	bytes_at: HashMap<Vec<u8>, (u32, u32)>,
}

impl StrPool {
	fn intern(&mut self, s: &str) -> (u32, u32) {
		if let Some(&p) = self.at.get(s) {
			return p;
		}
		let off = self.bytes.len() as u32;
		self.bytes.extend_from_slice(s.as_bytes());
		let p = (off, s.len() as u32);
		self.at.insert(s.to_string(), p);
		p
	}

	/// Intern a raw byte sequence (a `bytes` literal — not necessarily UTF-8).
	fn intern_bytes(&mut self, b: &[u8]) -> (u32, u32) {
		if let Some(&p) = self.bytes_at.get(b) {
			return p;
		}
		let off = self.bytes.len() as u32;
		self.bytes.extend_from_slice(b);
		let p = (off, b.len() as u32);
		self.bytes_at.insert(b.to_vec(), p);
		p
	}
}

fn scan_strings(b: &Block, pool: &mut StrPool, enums: &EnumTable) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => scan_rvalue_strings(rv, pool, enums),
			StmtKind::Return(a) | StmtKind::PushDefer(a) => scan_atom_string(a, pool),
			StmtKind::If(_, t, e) => {
				scan_strings(t, pool, enums);
				scan_strings(e, pool, enums);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					scan_strings(b, pool, enums);
				}
				scan_strings(default, pool, enums);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					scan_pattern_names(&a.pattern, pool);
					scan_strings(&a.body, pool, enums);
				}
			}
			StmtKind::Loop(b) => scan_strings(b, pool, enums),
			_ => {}
		}
	}
}

fn scan_atom_string(a: &Atom, pool: &mut StrPool) {
	match a {
		Atom::Const(Const::Str(s)) => {
			pool.intern(s);
		}
		Atom::Const(Const::Bytes(b)) => {
			pool.intern_bytes(b);
		}
		_ => {}
	}
}

fn scan_rvalue_strings(rv: &Rvalue, pool: &mut StrPool, enums: &EnumTable) {
	for_each_atom(rv, &mut |a| scan_atom_string(a, pool));
	// Variant display names + record field names become `$str` constants (not atoms).
	match rv {
		Rvalue::MakeVariant { enum_name, tag, .. } => {
			pool.intern(&variant_display(enum_name, *tag, enums));
		}
		Rvalue::MakeVariantCtor { enum_name, tag } => {
			pool.intern(&variant_display(enum_name, *tag, enums));
		}
		_ => {}
	}
	// Record field names become `$str` constants too (not atoms).
	match rv {
		Rvalue::MakeRecord(fields) | Rvalue::RecordUpdate { fields, .. } => {
			for (n, _) in fields {
				pool.intern(n);
			}
		}
		Rvalue::GetField(_, name) => {
			pool.intern(name);
		}
		_ => {}
	}
}

/// Intern record-pattern field names (matched via `__getfield`, so they need
/// `$str` constants).
fn scan_pattern_names(p: &ir::Pattern, pool: &mut StrPool) {
	match p {
		ir::Pattern::Record { fields, .. } => {
			for (n, sub) in fields {
				pool.intern(n);
				scan_pattern_names(sub, pool);
			}
		}
		ir::Pattern::Variant { fields, .. } | ir::Pattern::Tuple(fields) => {
			fields.iter().for_each(|p| scan_pattern_names(p, pool))
		}
		ir::Pattern::List { items, .. } => items.iter().for_each(|p| scan_pattern_names(p, pool)),
		_ => {}
	}
}

/// Visit every `Atom` operand of an rvalue (exhaustive — so the string-constant
/// pre-scan never misses one, whatever the construct).
fn for_each_atom(rv: &Rvalue, f: &mut impl FnMut(&Atom)) {
	use ir::ListItem;
	match rv {
		Rvalue::Use(a)
		| Rvalue::Not(a)
		| Rvalue::Box(a)
		| Rvalue::Unbox(a, _)
		| Rvalue::GetDictMethod(a, _)
		| Rvalue::GetField(a, _)
		| Rvalue::GetElement(a, _)
		| Rvalue::GetTag(a)
		| Rvalue::GetPayload(a, _)
		| Rvalue::Await(a) => f(a),
		Rvalue::Bin(_, a, b) => {
			f(a);
			f(b);
		}
		Rvalue::Call(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => args.iter().for_each(f),
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			f(c);
			args.iter().for_each(f);
		}
		Rvalue::MakeRecord(fields) => fields.iter().for_each(|(_, a)| f(a)),
		Rvalue::RecordUpdate { base, fields } => {
			f(base);
			fields.iter().for_each(|(_, a)| f(a));
		}
		Rvalue::MakeVariant { payload, .. } => payload.iter().for_each(f),
		Rvalue::MakeList(items) => items.iter().for_each(|it| match it {
			ListItem::Elem(a) | ListItem::Spread(a) => f(a),
		}),
		Rvalue::MakeVariantCtor { .. }
		| Rvalue::Regex(_)
		| Rvalue::GlobalRef(_)
		| Rvalue::Builtin(_) => {}
	}
}

/// Visit each builtin tag called (via a `GlobalRef`-to-builtin callee) in a block.
fn collect_host_calls(b: &Block, builtin_g: &HashMap<u32, String>, mut f: impl FnMut(&str)) {
	// First map local vars to builtin tags within this block scope.
	let var_tags = builtin_var_tags(b, builtin_g);
	collect_inner(b, &var_tags, &mut f);
}

fn collect_inner(b: &Block, var_tags: &HashMap<u32, String>, f: &mut impl FnMut(&str)) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Some(tag) = callee_builtin_tag(rv, var_tags) {
					f(tag);
				}
			}
			StmtKind::If(_, t, e) => {
				collect_inner(t, var_tags, f);
				collect_inner(e, var_tags, f);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_inner(b, var_tags, f);
				}
				collect_inner(default, var_tags, f);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_inner(&a.body, var_tags, f);
				}
			}
			StmtKind::Loop(b) => collect_inner(b, var_tags, f),
			_ => {}
		}
	}
}

/// Map a function's local vars to the builtin tag they hold, from
/// `Let(v, GlobalRef(g))` where `g` is a `PreEvaluated(Builtin)`. Recurses into
/// nested blocks (a single var-id namespace per function).
fn builtin_var_tags(b: &Block, builtin_g: &HashMap<u32, String>) -> HashMap<u32, String> {
	let mut m = HashMap::new();
	fn walk(b: &Block, builtin_g: &HashMap<u32, String>, m: &mut HashMap<u32, String>) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, Rvalue::GlobalRef(g)) => {
					if let Some(tag) = builtin_g.get(&g.0) {
						m.insert(v.0, tag.clone());
					}
				}
				StmtKind::If(_, t, e) => {
					walk(t, builtin_g, m);
					walk(e, builtin_g, m);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, b) in arms {
						walk(b, builtin_g, m);
					}
					walk(default, builtin_g, m);
				}
				StmtKind::Match { arms, .. } => {
					for a in arms {
						walk(&a.body, builtin_g, m);
					}
				}
				StmtKind::Loop(b) => walk(b, builtin_g, m),
				_ => {}
			}
		}
	}
	walk(b, builtin_g, &mut m);
	m
}

/// The display name of a variant — `bare-enum.variant`, matching `vm::Value`'s
/// `Display` (e.g. `tree.node`, `color.red`). Stored in each `$variant` so the
/// host formatter and `__tostring` can render it without a name table.
fn variant_display(enum_name: &str, tag: u32, enums: &EnumTable) -> String {
	let bare = enum_name.rsplit_once('.').map_or(enum_name, |(_, n)| n);
	let variant = enums
		.get(enum_name)
		.and_then(|vs| vs.get(tag as usize))
		.map_or("?", |(n, _)| n.as_str());
	format!("{bare}.{variant}")
}

/// Map a function's local vars to the `(enum_name, variant tag)` they hold, from
/// `Let(v, MakeVariantCtor{..})`. Recurses into nested blocks.
fn ctor_var_tags(b: &Block) -> HashMap<u32, (String, u32)> {
	let mut m = HashMap::new();
	fn walk(b: &Block, m: &mut HashMap<u32, (String, u32)>) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, Rvalue::MakeVariantCtor { enum_name, tag }) => {
					m.insert(v.0, (enum_name.clone(), *tag));
				}
				StmtKind::If(_, t, e) => {
					walk(t, m);
					walk(e, m);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, b) in arms {
						walk(b, m);
					}
					walk(default, m);
				}
				StmtKind::Match { arms, .. } => {
					for a in arms {
						walk(&a.body, m);
					}
				}
				StmtKind::Loop(b) => walk(b, m),
				_ => {}
			}
		}
	}
	walk(b, &mut m);
	m
}

/// Collect `MakeClosure` targets that have zero IR params (the `fun { }` form,
/// typed `nothing -> a` — arity 1 at every call site).
fn collect_zero_arg_closures(b: &Block, p: &IrProgram, out: &mut HashSet<u32>) {
	fn rv(rv: &Rvalue, p: &IrProgram, out: &mut HashSet<u32>) {
		if let Rvalue::MakeClosure(fid, _) = rv {
			if p.functions[fid.0 as usize].params.is_empty() {
				out.insert(fid.0);
			}
		}
	}
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, r) | StmtKind::Discard(r) => rv(r, p, out),
			StmtKind::If(_, t, e) => {
				collect_zero_arg_closures(t, p, out);
				collect_zero_arg_closures(e, p, out);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_zero_arg_closures(b, p, out);
				}
				collect_zero_arg_closures(default, p, out);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_zero_arg_closures(&a.body, p, out);
				}
			}
			StmtKind::Loop(b) => collect_zero_arg_closures(b, p, out),
			_ => {}
		}
	}
}

fn callee_builtin_tag<'a>(rv: &Rvalue, var_tags: &'a HashMap<u32, String>) -> Option<&'a str> {
	let callee = match rv {
		Rvalue::CallClosure(c, _) | Rvalue::TailCall(c, _) => c,
		_ => return None,
	};
	if let Atom::Var(v) = callee {
		var_tags.get(&v.0).map(|s| s.as_str())
	} else {
		None
	}
}

// --------------------------------------------------------------------------
// Per-function body emission.
// --------------------------------------------------------------------------

type EnumTable = HashMap<String, Vec<(String, usize)>>;

struct FnEmitter<'a> {
	f: &'a ir::Function,
	wasm_index: &'a HashMap<u32, u32>,
	host_index: &'a HashMap<String, u32>,
	gmap: &'a HashMap<u32, GlobalSlot>,
	runtime: Runtime,
	enums: &'a EnumTable,
	ftypes: &'a mut FuncTypes,
	var_tags: HashMap<u32, String>,
	/// VarId.0 -> variant tag, for vars bound to a `MakeVariantCtor`. Applying
	/// such a value (a `CallClosure` on it) builds the variant directly.
	var_ctors: HashMap<u32, (String, u32)>,
	strpool: &'a StrPool,
	diags: &'a mut Diagnostics,
	/// VarId.0 -> wasm local index. Wasm local 0 is the implicit closure env.
	locals: Vec<u32>,
	/// Local types for the locals past the wasm params, in declaration order.
	local_types: Vec<ValType>,
	/// Next free wasm local index (params occupy `0..=arity`).
	next_local: u32,
	/// Current control-flow nesting depth, for relative `br` targets.
	depth: u32,
	/// Enclosing loops as (continue-target level, break-target level).
	loop_stack: Vec<(u32, u32)>,
	body: Vec<Instruction<'static>>,
}

impl<'a> FnEmitter<'a> {
	#[allow(clippy::too_many_arguments)]
	fn new(
		f: &'a ir::Function,
		wasm_index: &'a HashMap<u32, u32>,
		host_index: &'a HashMap<String, u32>,
		builtin_g: &HashMap<u32, String>,
		gmap: &'a HashMap<u32, GlobalSlot>,
		runtime: &Runtime,
		strpool: &'a StrPool,
		enums: &'a EnumTable,
		ftypes: &'a mut FuncTypes,
		extra_params: u32,
		diags: &'a mut Diagnostics,
	) -> Self {
		let var_tags = builtin_var_tags(&f.body, builtin_g);
		let var_ctors = ctor_var_tags(&f.body);
		let n = f.var_reprs.len().max(f.params.len() + f.captures.len());
		let mut locals = vec![u32::MAX; n];
		// Wasm params: local 0 = env (closure ref/null), then the source params,
		// then any phantom params (the `fun { }` unit arg, mapped to no VarId).
		for (i, p) in f.params.iter().enumerate() {
			locals[p.0 as usize] = (i + 1) as u32;
		}
		let mut local_types = Vec::new();
		let mut next = (f.params.len() + 1) as u32 + extra_params;
		// Captures get locals too; loaded from the env in the prologue.
		for c in &f.captures {
			locals[c.0 as usize] = next;
			next += 1;
			local_types.push(types::value_ref());
		}
		// Every other var gets a fresh local, typed by its repr.
		for v in 0..n {
			if locals[v] == u32::MAX {
				locals[v] = next;
				next += 1;
				let repr = f.var_reprs.get(v).copied().unwrap_or(Repr::Boxed);
				local_types.push(repr_valtype(repr));
			}
		}
		Self {
			f,
			wasm_index,
			host_index,
			gmap,
			runtime: *runtime,
			enums,
			ftypes,
			var_tags,
			var_ctors,
			strpool,
			diags,
			locals,
			local_types,
			next_local: next,
			depth: 0,
			loop_stack: Vec::new(),
			body: Vec::new(),
		}
	}

	fn emit(&mut self) -> Function {
		// Prologue: copy each captured value out of the env (`$closure` captures
		// array) into its local, so capture vars read like any other local.
		let caps: Vec<u32> = self.f.captures.iter().map(|c| c.0).collect();
		for (i, c) in caps.into_iter().enumerate() {
			let dst = self.local(c);
			self.ins(Instruction::LocalGet(0));
			self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
				types::T_CLOSURE,
			)));
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_CLOSURE,
				field_index: 2,
			});
			self.ins(Instruction::I32Const(i as i32));
			self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			self.ins(Instruction::LocalSet(dst));
		}
		let body = self.f.body.clone();
		self.block(&body);
		let mut func = Function::new_with_locals_types(self.local_types.iter().copied());
		for ins in &self.body {
			func.instruction(ins);
		}
		func.instruction(&Instruction::End);
		func
	}

	/// Allocate a fresh wasm local of the given type, returning its index.
	fn fresh_local(&mut self, ty: ValType) -> u32 {
		let idx = self.next_local;
		self.next_local += 1;
		self.local_types.push(ty);
		idx
	}

	fn block(&mut self, b: &Block) {
		for s in &b.0 {
			self.stmt(&s.kind);
		}
	}

	fn stmt(&mut self, k: &StmtKind) {
		match k {
			StmtKind::Let(v, rv) => {
				self.rvalue(rv);
				self.ins(Instruction::LocalSet(self.local(v.0)));
			}
			StmtKind::Discard(rv) => {
				self.rvalue(rv);
				self.ins(Instruction::Drop);
			}
			StmtKind::Return(a) => {
				self.atom(a);
				self.ins(Instruction::Return);
			}
			StmtKind::If(cond, t, e) => {
				self.atom(cond);
				self.ins(Instruction::If(wasm_encoder::BlockType::Empty));
				self.depth += 1;
				self.block(t);
				self.ins(Instruction::Else);
				self.block(e);
				self.ins(Instruction::End);
				self.depth -= 1;
			}
			StmtKind::Loop(b) => {
				let break_level = self.open_block();
				let cont_level = self.depth;
				self.ins(Instruction::Loop(wasm_encoder::BlockType::Empty));
				self.depth += 1;
				self.loop_stack.push((cont_level, break_level));
				self.block(b);
				// Back-edge: re-iterate the loop (exited via `Break`).
				self.ins(Instruction::Br(self.br_to(cont_level)));
				self.loop_stack.pop();
				self.ins(Instruction::End);
				self.depth -= 1;
				self.close_block();
			}
			StmtKind::Break => match self.loop_stack.last() {
				Some(&(_, brk)) => self.ins(Instruction::Br(self.br_to(brk))),
				None => self.diags.push("Break outside loop"),
			},
			StmtKind::Continue => match self.loop_stack.last() {
				Some(&(cont, _)) => self.ins(Instruction::Br(self.br_to(cont))),
				None => self.diags.push("Continue outside loop"),
			},
			StmtKind::Match { subject, arms } => self.match_stmt(subject, arms),
			other => self.diags.push(format!("unsupported stmt: {other:?}")),
		}
	}

	fn open_block(&mut self) -> u32 {
		let lvl = self.depth;
		self.ins(Instruction::Block(wasm_encoder::BlockType::Empty));
		self.depth += 1;
		lvl
	}

	fn close_block(&mut self) {
		self.ins(Instruction::End);
		self.depth -= 1;
	}

	/// The relative `br` immediate that targets the construct opened at `level`.
	fn br_to(&self, level: u32) -> u32 {
		self.depth - level - 1
	}

	/// Lower a pattern `Match`: evaluate the subject once, then try each arm in a
	/// nested block — a pattern mismatch `br`s past that arm to the next; a match
	/// runs the body and `br`s to the end (skipping later arms). No value is left
	/// on the stack (arms set locals or `Return`); the join, if any, is a local.
	fn match_stmt(&mut self, subject: &Atom, arms: &[ir::MatchArm]) {
		let subj = self.fresh_local(types::value_ref());
		self.atom(subject);
		self.ins(Instruction::LocalSet(subj));
		let end_level = self.open_block();
		for arm in arms {
			let arm_level = self.open_block();
			self.test_pattern(&arm.pattern, subj, arm_level);
			self.block(&arm.body);
			self.ins(Instruction::Br(self.br_to(end_level)));
			self.close_block();
		}
		self.close_block();
	}

	/// Test `pat` against the value in local `subj`. On mismatch, `br` to the
	/// block opened at `fail_level`. On match, bind any sub-vars.
	fn test_pattern(&mut self, pat: &ir::Pattern, subj: u32, fail_level: u32) {
		use ir::Pattern::*;
		match pat {
			Wildcard => {}
			Bind(v) => {
				let dst = self.local(v.0);
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::LocalSet(dst));
			}
			Literal(c) => self.test_literal(c, subj, fail_level),
			Variant { variant, fields } => self.test_variant(variant, fields, subj, fail_level),
			Tuple(elems) => {
				// A tuple's arity is fixed by its type — no tag/length check.
				for (i, sub) in elems.iter().enumerate() {
					self.bind_at(sub, subj, types::T_TUPLE, 1, i, fail_level);
				}
			}
			List { items, rest } => {
				// Length: exact (== items) when no rest, else at-least (>= items).
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I32Const(items.len() as i32));
				if rest.is_some() {
					self.ins(Instruction::I32LtS); // len < items -> fail
				} else {
					self.ins(Instruction::I32Ne); // len != items -> fail
				}
				self.ins(Instruction::BrIf(self.br_to(fail_level)));
				for (i, sub) in items.iter().enumerate() {
					self.bind_at(sub, subj, types::T_LIST, 1, i, fail_level);
				}
				if let Some(ir::ListRest::Bind(v)) = rest {
					// rest = __list_tail(list, items.len()).
					let tail = self.runtime.list_tail_fn.expect("list_tail");
					let dst = self.local(v.0);
					self.ins(Instruction::LocalGet(subj));
					self.ins(Instruction::I32Const(types::TAG_INT));
					self.ins(Instruction::I64Const(items.len() as i64));
					self.ins(Instruction::StructNew(types::T_INT));
					self.ins(Instruction::Call(tail));
					self.ins(Instruction::LocalSet(dst));
				}
			}
			Record { fields, rest } => {
				if let ir::RecordRest::Exact = rest {
					self.ins(Instruction::LocalGet(subj));
					self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
						types::T_RECORD,
					)));
					self.ins(Instruction::StructGet {
						struct_type_index: types::T_RECORD,
						field_index: 1,
					});
					self.ins(Instruction::ArrayLen);
					self.ins(Instruction::I32Const(fields.len() as i32));
					self.ins(Instruction::I32Ne);
					self.ins(Instruction::BrIf(self.br_to(fail_level)));
				}
				if let ir::RecordRest::Bind(_) = rest {
					self
						.diags
						.push("record `...rest` binding not yet supported");
					return;
				}
				let getfield = self.runtime.getfield_fn.expect("getfield");
				for (name, sub) in fields {
					match sub {
						ir::Pattern::Wildcard => {}
						_ => {
							let tmp = self.fresh_local(types::value_ref());
							self.ins(Instruction::LocalGet(subj));
							self.string_const(name);
							self.ins(Instruction::Call(getfield));
							self.ins(Instruction::LocalSet(tmp));
							self.test_pattern(sub, tmp, fail_level);
						}
					}
				}
			}
		}
	}

	/// Match sub-pattern `sub` against element `i` of `subj` (a struct of type
	/// `sty` whose field `field` is the `$valarray`). Binds/recurses; on mismatch
	/// `br`s to `fail_level`.
	fn bind_at(&mut self, sub: &ir::Pattern, subj: u32, sty: u32, field: u32, i: usize, fail: u32) {
		match sub {
			ir::Pattern::Wildcard => {}
			ir::Pattern::Bind(v) => {
				let dst = self.local(v.0);
				self.get_elem(subj, sty, field, i);
				self.ins(Instruction::LocalSet(dst));
			}
			other => {
				let tmp = self.fresh_local(types::value_ref());
				self.get_elem(subj, sty, field, i);
				self.ins(Instruction::LocalSet(tmp));
				self.test_pattern(other, tmp, fail);
			}
		}
	}

	/// Push element `i` of the `$valarray` in field `field` of struct `subj:sty`.
	fn get_elem(&mut self, subj: u32, sty: u32, field: u32, i: usize) {
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(sty)));
		self.ins(Instruction::StructGet {
			struct_type_index: sty,
			field_index: field,
		});
		self.ins(Instruction::I32Const(i as i32));
		self.ins(Instruction::ArrayGet(types::T_VALARRAY));
	}

	fn test_literal(&mut self, c: &Const, subj: u32, fail_level: u32) {
		let br = self.br_to(fail_level);
		match c {
			Const::Bool(b) => {
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_BOOL,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_BOOL,
					field_index: 1,
				});
				self.ins(Instruction::I32Const(*b as i32));
				self.ins(Instruction::I32Ne);
				self.ins(Instruction::BrIf(br));
			}
			Const::Int(n) => {
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I64Const(*n));
				self.ins(Instruction::I64Ne);
				self.ins(Instruction::BrIf(br));
			}
			Const::Float(x) => {
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_FLOAT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_FLOAT,
					field_index: 1,
				});
				self.ins(Instruction::F64Const((*x).into()));
				self.ins(Instruction::F64Ne);
				self.ins(Instruction::BrIf(br));
			}
			other => self
				.diags
				.push(format!("unsupported literal pattern: {other:?}")),
		}
	}

	fn test_variant(&mut self, name: &str, fields: &[ir::Pattern], subj: u32, fail_level: u32) {
		let Some(tag) = self.variant_tag(name) else {
			self.diags.push(format!("cannot resolve variant `{name}`"));
			return;
		};
		// Tag check.
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_VARIANT,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_VARIANT,
			field_index: 1,
		});
		self.ins(Instruction::I32Const(tag as i32));
		self.ins(Instruction::I32Ne);
		self.ins(Instruction::BrIf(self.br_to(fail_level)));
		// Bind / recurse on payload fields (variant payload is field 3).
		for (i, field) in fields.iter().enumerate() {
			self.bind_at(field, subj, types::T_VARIANT, 3, i, fail_level);
		}
	}

	/// Resolve a variant name to its within-enum tag. Sound when the name is
	/// unique across enums, or all its occurrences share a tag (the within-match
	/// enum is fixed by the type system, so a same-tag collision is harmless).
	fn variant_tag(&self, name: &str) -> Option<u32> {
		let mut found: Option<u32> = None;
		for variants in self.enums.values() {
			if let Some(i) = variants.iter().position(|(n, _)| n == name) {
				match found {
					None => found = Some(i as u32),
					Some(t) if t == i as u32 => {}
					Some(_) => return None, // ambiguous: differing tags
				}
			}
		}
		found
	}

	fn rvalue(&mut self, rv: &Rvalue) {
		match rv {
			Rvalue::Use(a) => self.atom(a),
			Rvalue::Bin(op @ (ir::BinOp::Eq | ir::BinOp::Ne), a, b) => {
				let Some(eq) = self.runtime.eq_fn else {
					self.diags.push("Eq/Ne used but __eq not emitted");
					return;
				};
				self.atom(a);
				self.atom(b);
				self.ins(Instruction::Call(eq));
				if matches!(op, ir::BinOp::Ne) {
					self.ins(Instruction::I32Eqz);
				}
			}
			Rvalue::Bin(ir::BinOp::Concat, a, b) => {
				// `++`: concatenate two strings' byte arrays, rewrap as `$str`.
				let Some(bc) = self.runtime.bytesconcat_fn else {
					self.diags.push("Concat used but __bytesconcat not emitted");
					return;
				};
				self.str_bytes(a);
				self.str_bytes(b);
				self.ins(Instruction::Call(bc));
				let tmp = self.fresh_local(types::bytes_ref());
				self.ins(Instruction::LocalSet(tmp));
				self.ins(Instruction::I32Const(types::TAG_STR));
				self.ins(Instruction::LocalGet(tmp));
				self.ins(Instruction::StructNew(types::T_STR));
			}
			Rvalue::Bin(op, a, b) => {
				self.atom(a);
				self.atom(b);
				match binop_instr(*op) {
					Some(ins) => self.ins(ins),
					None => self.diags.push(format!("unsupported binop: {op:?}")),
				}
			}
			Rvalue::Interpolate(parts) => {
				// Parts are already strings (the analyzer inserts `to-string`); fold
				// their byte arrays with `__bytesconcat`, rewrap as `$str`.
				if parts.is_empty() {
					self.ins(Instruction::I32Const(types::TAG_STR));
					self.ins(Instruction::ArrayNewFixed {
						array_type_index: types::T_BYTES,
						array_size: 0,
					});
					self.ins(Instruction::StructNew(types::T_STR));
					return;
				}
				let Some(bc) = self.runtime.bytesconcat_fn else {
					self
						.diags
						.push("Interpolate used but __bytesconcat not emitted");
					return;
				};
				for (i, part) in parts.iter().enumerate() {
					self.str_bytes(part);
					if i > 0 {
						self.ins(Instruction::Call(bc));
					}
				}
				let tmp = self.fresh_local(types::bytes_ref());
				self.ins(Instruction::LocalSet(tmp));
				self.ins(Instruction::I32Const(types::TAG_STR));
				self.ins(Instruction::LocalGet(tmp));
				self.ins(Instruction::StructNew(types::T_STR));
			}
			Rvalue::Not(a) => {
				// `!b` over an i32 boolean: b == 0.
				self.atom(a);
				self.ins(Instruction::I32Eqz);
			}
			Rvalue::Box(a) => {
				let repr = self.atom_repr(a);
				let (tag, ty) = match repr {
					Repr::I64 => (types::TAG_INT, types::T_INT),
					Repr::F64 => (types::TAG_FLOAT, types::T_FLOAT),
					Repr::I32 => (types::TAG_BOOL, types::T_BOOL),
					Repr::Boxed => {
						self.diags.push("Box of an already-boxed value");
						return;
					}
				};
				self.ins(Instruction::I32Const(tag));
				self.atom(a);
				self.ins(Instruction::StructNew(ty));
			}
			Rvalue::Unbox(a, repr) => {
				self.atom(a);
				let (ty, field) = match repr {
					Repr::I64 => (types::T_INT, 1),
					Repr::F64 => (types::T_FLOAT, 1),
					Repr::I32 => (types::T_BOOL, 1),
					Repr::Boxed => {
						self.diags.push("Unbox to Boxed");
						return;
					}
				};
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(ty)));
				self.ins(Instruction::StructGet {
					struct_type_index: ty,
					field_index: field,
				});
			}
			Rvalue::Call(Callee::Function(fid), args) => {
				let Some(&w) = self.wasm_index.get(&fid.0) else {
					self.diags.push(format!("call to unreachable fn {}", fid.0));
					self.push_nothing();
					return;
				};
				// A direct call targets a capture-free function: pass a null env.
				self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
				for a in args {
					self.atom(a);
				}
				self.ins(Instruction::Call(w));
			}
			Rvalue::CallClosure(callee, args) => self.call_value(callee, args, false),
			Rvalue::TailCall(callee, args) => self.call_value(callee, args, true),
			Rvalue::MakeClosure(fid, caps) => {
				let Some(&w) = self.wasm_index.get(&fid.0) else {
					self
						.diags
						.push(format!("closure over unreachable fn {}", fid.0));
					self.push_nothing();
					return;
				};
				self.ins(Instruction::I32Const(types::TAG_CLOSURE));
				self.ins(Instruction::I32Const(w as i32));
				for a in caps {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: caps.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_CLOSURE));
			}
			Rvalue::MakeVariant {
				enum_name,
				tag,
				payload,
			} => {
				self.ins(Instruction::I32Const(types::TAG_VARIANT));
				self.ins(Instruction::I32Const(*tag as i32));
				self.string_const(&variant_display(enum_name, *tag, self.enums));
				for a in payload {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: payload.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_VARIANT));
			}
			Rvalue::MakeVariantCtor { tag, enum_name } => {
				let arity = self.variant_arity(enum_name, *tag);
				self.ins(Instruction::I32Const(types::TAG_CTOR));
				self.ins(Instruction::I32Const(*tag as i32));
				self.ins(Instruction::I32Const(arity as i32));
				self.ins(Instruction::StructNew(types::T_CTOR));
			}
			Rvalue::MakeTuple(elems) => {
				self.ins(Instruction::I32Const(types::TAG_TUPLE));
				self.elems_array(elems);
				self.ins(Instruction::StructNew(types::T_TUPLE));
			}
			Rvalue::MakeList(items) => self.make_list(items),
			Rvalue::MakeRecord(fields) => {
				// Sort by field name for a canonical layout; names + values parallel.
				let mut sorted: Vec<(&String, &Atom)> = fields.iter().map(|(n, a)| (n, a)).collect();
				sorted.sort_by(|a, b| a.0.cmp(b.0));
				self.ins(Instruction::I32Const(types::TAG_RECORD));
				for (n, _) in &sorted {
					self.string_const(n);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: sorted.len() as u32,
				});
				for (_, a) in &sorted {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: sorted.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_RECORD));
			}
			Rvalue::GetField(r, name) => {
				let Some(getfield) = self.runtime.getfield_fn else {
					self.diags.push("GetField used but __getfield not emitted");
					return;
				};
				self.atom(r);
				self.string_const(name);
				self.ins(Instruction::Call(getfield));
			}
			Rvalue::RecordUpdate { base, fields } => {
				let Some(update) = self.runtime.record_update_fn else {
					self
						.diags
						.push("RecordUpdate used but __record_update not emitted");
					return;
				};
				// __record_update(base, name, value) applied once per override.
				self.atom(base);
				for (n, a) in fields {
					self.string_const(n);
					self.atom(a);
					self.ins(Instruction::Call(update));
				}
			}
			Rvalue::MakeDict(methods) => {
				self.ins(Instruction::I32Const(types::TAG_METHODDICT));
				for a in methods {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: methods.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_METHODDICT));
			}
			Rvalue::GetDictMethod(dict, idx) => {
				self.atom(dict);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_METHODDICT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_METHODDICT,
					field_index: 1,
				});
				self.ins(Instruction::I32Const(*idx as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GetTag(a) => {
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_VARIANT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_VARIANT,
					field_index: 1,
				});
			}
			Rvalue::GetPayload(a, i) => {
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_VARIANT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_VARIANT,
					field_index: 3, // payload (after tag, vtag, name)
				});
				self.ins(Instruction::I32Const(*i as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GetElement(a, i) => {
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_TUPLE,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_TUPLE,
					field_index: 1, // elems array (after tag)
				});
				self.ins(Instruction::I32Const(*i as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GlobalRef(g) => {
				if let Some(slot) = self.gmap.get(&g.0).cloned() {
					// Lazy: build the value once, cache behind the init flag, then load.
					self.ins(Instruction::GlobalGet(slot.init_idx));
					self.ins(Instruction::I32Eqz);
					self.ins(Instruction::If(wasm_encoder::BlockType::Empty));
					self.depth += 1;
					match &slot.kind {
						GlobalKind::Thunk(thunk_wasm) => {
							self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE))); // env
							self.ins(Instruction::Call(*thunk_wasm));
						}
						GlobalKind::MethodDict(wrappers) => {
							// Build a $methoddict of capture-free wrapper closures.
							self.ins(Instruction::I32Const(types::TAG_METHODDICT));
							for &w in wrappers {
								self.ins(Instruction::I32Const(types::TAG_CLOSURE));
								self.ins(Instruction::I32Const(w as i32));
								self.ins(Instruction::ArrayNewFixed {
									array_type_index: types::T_VALARRAY,
									array_size: 0,
								});
								self.ins(Instruction::StructNew(types::T_CLOSURE));
							}
							self.ins(Instruction::ArrayNewFixed {
								array_type_index: types::T_VALARRAY,
								array_size: wrappers.len() as u32,
							});
							self.ins(Instruction::StructNew(types::T_METHODDICT));
						}
					}
					self.ins(Instruction::GlobalSet(slot.val_idx));
					self.ins(Instruction::I32Const(1));
					self.ins(Instruction::GlobalSet(slot.init_idx));
					self.ins(Instruction::End);
					self.depth -= 1;
					self.ins(Instruction::GlobalGet(slot.val_idx));
				} else {
					// A builtin-global reference used only as a call target: emit a null
					// placeholder (its only consumer is the call site, special-cased).
					self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
				}
			}
			other => self.diags.push(format!("unsupported rvalue: {other:?}")),
		}
	}

	/// Dispatch a `CallClosure`/`TailCall` by callee kind: host builtin, a partial
	/// variant constructor (build the variant), or a runtime closure.
	fn call_value(&mut self, callee: &Atom, args: &[Atom], tail: bool) {
		if let Some(tag) = self.callee_tag(callee) {
			self.host_call(&tag, args);
			return;
		}
		if let Atom::Var(v) = callee {
			if let Some((enum_name, tag)) = self.var_ctors.get(&v.0).cloned() {
				// Applying a constructor builds the variant directly.
				self.ins(Instruction::I32Const(types::TAG_VARIANT));
				self.ins(Instruction::I32Const(tag as i32));
				self.string_const(&variant_display(&enum_name, tag, self.enums));
				for a in args {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: args.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_VARIANT));
				return;
			}
		}
		self.closure_call(callee, args, tail);
	}

	/// `CallClosure`/`TailCall` on a runtime closure value: pass the closure as
	/// the env (param 0), then the args, then `call_indirect` through its stored
	/// `fn_index`.
	fn closure_call(&mut self, callee: &Atom, args: &[Atom], tail: bool) {
		let arity = args.len();
		// env = the closure value.
		self.atom(callee);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_CLOSURE,
		)));
		for a in args {
			self.atom(a);
		}
		// fn_index from the closure.
		self.atom(callee);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_CLOSURE,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_CLOSURE,
			field_index: 1,
		});
		let ty = self.ftypes.for_arity(arity);
		if tail {
			self.ins(Instruction::ReturnCallIndirect {
				type_index: ty,
				table_index: 0,
			});
		} else {
			self.ins(Instruction::CallIndirect {
				type_index: ty,
				table_index: 0,
			});
		}
	}

	fn variant_arity(&self, enum_name: &str, tag: u32) -> usize {
		self
			.enums
			.get(enum_name)
			.and_then(|vs| vs.get(tag as usize))
			.map(|(_, a)| *a)
			.unwrap_or(0)
	}

	/// The builtin tag a callee atom resolves to, if any.
	fn callee_tag(&self, callee: &Atom) -> Option<String> {
		if let Atom::Var(v) = callee {
			self.var_tags.get(&v.0).cloned()
		} else {
			None
		}
	}

	fn host_call(&mut self, tag: &str, args: &[Atom]) {
		// Pure-compute builtins emitted inline over the `$value` GC layout.
		if is_inline_builtin(tag) {
			self.inline_builtin(tag, args);
			return;
		}
		// Unary float math (log/exp/sin/cos): unbox the `$float`, call the raw
		// `(f64) -> f64` host import, rebox. Keeps the GC poking in wasm.
		if is_f64_unary_host(tag) {
			match self.host_index.get(tag).copied() {
				Some(idx) => {
					self.ins(Instruction::I32Const(types::TAG_FLOAT));
					self.atom(&args[0]);
					self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
						types::T_FLOAT,
					)));
					self.ins(Instruction::StructGet {
						struct_type_index: types::T_FLOAT,
						field_index: 1,
					});
					self.ins(Instruction::Call(idx));
					self.ins(Instruction::StructNew(types::T_FLOAT));
				}
				None => {
					self.diags.push(format!("`{tag}` host import not declared"));
					self.push_nothing();
				}
			}
			return;
		}
		// Higher-order builders: synthetic helpers (loop + closure call).
		if tag == "list-build" || tag == "list-collect" || tag == "bytes-build" {
			let helper = match tag {
				"list-build" => self.runtime.list_build_fn,
				"list-collect" => self.runtime.list_collect_fn,
				_ => self.runtime.bytes_build_fn,
			};
			match helper {
				Some(h) => {
					for a in args {
						self.atom(a);
					}
					self.ins(Instruction::Call(h));
				}
				None => {
					self.diags.push(format!("`{tag}` helper not emitted"));
					self.push_nothing();
				}
			}
			return;
		}
		// bytes.concat a b : a fresh `bytes` of a's bytes then b's, via __bytesconcat.
		if tag == "bytes-concat" {
			match self.runtime.bytesconcat_fn {
				Some(bc) => {
					self.ins(Instruction::I32Const(types::TAG_BYTES));
					self.str_bytes(&args[0]);
					self.str_bytes(&args[1]);
					self.ins(Instruction::Call(bc));
					self.ins(Instruction::StructNew(types::T_STR));
				}
				None => {
					self
						.diags
						.push("bytes-concat needs __bytesconcat".to_string());
					self.push_nothing();
				}
			}
			return;
		}
		// dict scan/rebuild/closure ops → synthetic helpers. insert/lookup/remove
		// receive a hash method-dict as `args[0]` (the `where (hash k)` evidence);
		// the wasm dict scans with `__eq` instead of hashing, so that arg is DROPPED
		// — we pass only the dict + key (+ value). map/filter take `[dict, f]`.
		if let Some((helper, call_args)) = match tag {
			"dict-insert" => Some((self.runtime.dict_insert_fn, &args[1..])),
			"dict-lookup" => Some((self.runtime.dict_lookup_fn, &args[1..])),
			"dict-remove" => Some((self.runtime.dict_remove_fn, &args[1..])),
			"dict-map" => Some((self.runtime.dict_map_fn, &args[0..])),
			"dict-filter" => Some((self.runtime.dict_filter_fn, &args[0..])),
			_ => None,
		} {
			match helper {
				Some(h) => {
					for a in call_args {
						self.atom(a);
					}
					self.ins(Instruction::Call(h));
				}
				None => {
					self.diags.push(format!("`{tag}` helper not emitted"));
					self.push_nothing();
				}
			}
			return;
		}
		// `wire-fingerprint`: FNV-1a hash of the schema tree, boxed as `$int`.
		if tag == "wire-fingerprint" {
			match self.runtime.wire_fp_fn {
				Some(fp) => {
					self.ins(Instruction::I32Const(types::TAG_INT));
					self.ins(Instruction::I64Const(WIRE_FNV_OFFSET));
					self.atom(&args[0]);
					self.ins(Instruction::Call(fp));
					self.ins(Instruction::StructNew(types::T_INT));
					return;
				}
				None => {
					self.diags.push("wire-fingerprint needs __wire_fp".to_string());
					self.push_nothing();
					return;
				}
			}
		}
		// `to-string` is implemented in wasm (`__tostring`), not imported.
		if tag == "to-string" {
			if let (Some(ts), Some(a)) = (self.runtime.tostring_fn, args.first()) {
				self.atom(a);
				self.ins(Instruction::Call(ts));
				return;
			}
			self.diags.push("to-string used but __tostring not emitted");
			self.push_nothing();
			return;
		}
		let Some(&idx) = self.host_index.get(tag) else {
			self.diags.push(format!("unsupported host builtin `{tag}`"));
			self.push_nothing();
			return;
		};
		for a in args {
			self.atom(a);
		}
		self.ins(Instruction::Call(idx));
		// `print`/`debug` return nothing; the `Let` binding expects a value, so
		// materialize `nothing`.
		if !host_sig(tag).map(|s| s.returns_value).unwrap_or(true) {
			self.push_nothing();
		}
	}

	/// Emit a pure-compute builtin inline over the `$value` GC layout.
	/// Leaves one `$value` on the stack (the binding's rvalue).
	fn inline_builtin(&mut self, tag: &str, args: &[Atom]) {
		match tag {
			// list.get xs i : the i-th element. (`$int` index unboxed to i32.)
			"list-get" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			// list.set xs i v : overwrite the i-th slot in place; yields nothing.
			// The deliberate escape hatch from list immutability.
			"list-set" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I32WrapI64);
				self.atom(&args[2]);
				self.ins(Instruction::ArraySet(types::T_VALARRAY));
				self.push_nothing();
			}
			// ref.new v : a fresh `$ref` cell holding `v`.
			"ref-new" => {
				self.ins(Instruction::I32Const(types::TAG_REF));
				self.atom(&args[0]);
				self.ins(Instruction::StructNew(types::T_REF));
			}
			// ref.get r : the cell's current value.
			"ref-get" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_REF,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
			}
			// ref.set r v : overwrite the cell in place; yields nothing.
			"ref-set" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_REF,
				)));
				self.atom(&args[1]);
				self.ins(Instruction::StructSet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
				self.push_nothing();
			}
			// ref.update r f : read once, apply the closure `f`, write back; yields
			// nothing. The closure call is emitted inline (env + 1 arg, then
			// `call_indirect` through its stored fn_index), with the cell struct kept
			// underneath for the final `struct.set`.
			"ref-update" => {
				let cell = self.fresh_local(types::ref_ref());
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_REF,
				)));
				self.ins(Instruction::LocalTee(cell)); // stack: [cell]
				// closure env = f.
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_CLOSURE,
				)));
				// arg = the cell's current value.
				self.ins(Instruction::LocalGet(cell));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
				// fn_index from the closure, then call_indirect (arity 1).
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_CLOSURE,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_CLOSURE,
					field_index: 1,
				});
				let ty = self.ftypes.for_arity(1);
				self.ins(Instruction::CallIndirect {
					type_index: ty,
					table_index: 0,
				}); // stack: [cell, new_value]
				self.ins(Instruction::StructSet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
				self.push_nothing();
			}
			// dict.empty () : a fresh `$dict` with no entries. (arg is the unit.)
			"dict-empty" => {
				self.ins(Instruction::I32Const(types::TAG_DICT));
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: 0,
				});
				self.ins(Instruction::StructNew(types::T_DICT));
			}
			// dict.size m : entry count, boxed as `$int`.
			"dict-size" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_DICT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_DICT,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// dict.entries m : the (k,v) tuples as a `$list`. The dict's entries
			// array is already a `$valarray` of `$tuple`s — just retag it as a list
			// (shared; neither side mutates it in place).
			"dict-entries" => {
				self.ins(Instruction::I32Const(types::TAG_LIST));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_DICT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_DICT,
					field_index: 1,
				});
				self.ins(Instruction::StructNew(types::T_LIST));
			}
			// list.length xs : element count, boxed as `$int`.
			"list-length" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// bytes.get b i : the i-th byte (0-255) as `$int`. (`$bytes` is packed
			// i8, read unsigned.)
			"bytes-get" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::ArrayGetU(types::T_BYTES));
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// bytes.length b : byte count, boxed as `$int`.
			"bytes-length" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// bytes <-> string reinterpret: same `{tag, $bytes}` shape, just
			// rebuild the struct with the other tag (no copy, no validation).
			"bytes-as-string" | "string-to-bytes" => {
				let new_tag = if tag == "bytes-as-string" {
					types::TAG_STR
				} else {
					types::TAG_BYTES
				};
				self.ins(Instruction::I32Const(new_tag));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.ins(Instruction::StructNew(types::T_STR));
			}
			// math.sqrt f : unbox the f64, `f64.sqrt`, rebox. NaN for f < 0,
			// matching the IEEE-754 result the VM's `f64::sqrt` yields.
			"math-sqrt" => {
				self.ins(Instruction::I32Const(types::TAG_FLOAT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_FLOAT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_FLOAT,
					field_index: 1,
				});
				self.ins(Instruction::F64Sqrt);
				self.ins(Instruction::StructNew(types::T_FLOAT));
			}
			// math.to-int f : truncate toward zero into an i64. The *saturating*
			// trunc matches the VM's `f as i64` (NaN -> 0, ±inf / out-of-range
			// clamp to i64::MIN/MAX); plain `i64.trunc_f64_s` would trap instead.
			"math-to-int" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_FLOAT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_FLOAT,
					field_index: 1,
				});
				self.ins(Instruction::I64TruncSatF64S);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// math.to-float n : widen the i64 to f64.
			"math-to-float" => {
				self.ins(Instruction::I32Const(types::TAG_FLOAT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::F64ConvertI64S);
				self.ins(Instruction::StructNew(types::T_FLOAT));
			}
			_ => {
				self
					.diags
					.push(format!("inline builtin `{tag}` not emitted"));
				self.push_nothing();
			}
		}
	}

	fn atom(&mut self, a: &Atom) {
		match a {
			Atom::Var(v) => self.ins(Instruction::LocalGet(self.local(v.0))),
			Atom::Const(c) => self.constant(c),
		}
	}

	fn constant(&mut self, c: &Const) {
		match c {
			Const::Int(n) => self.ins(Instruction::I64Const(*n)),
			Const::Float(x) => self.ins(Instruction::F64Const((*x).into())),
			Const::Bool(b) => self.ins(Instruction::I32Const(*b as i32)),
			Const::Unit => self.push_nothing(),
			Const::Str(s) => self.string_const(s),
			Const::Duration(n) => {
				self.ins(Instruction::I32Const(types::TAG_DURATION));
				self.ins(Instruction::I64Const(*n));
				self.ins(Instruction::StructNew(types::T_INT));
			}
			Const::Bytes(b) => self.bytes_const(b),
		}
	}

	/// A `bytes` literal: the `$str`-shaped struct (`{tag, ref $bytes}`) tagged
	/// `TAG_BYTES`. Backing bytes come from the shared passive data segment.
	fn bytes_const(&mut self, b: &[u8]) {
		let Some(&(off, len)) = self.strpool.bytes_at.get(b) else {
			self
				.diags
				.push("bytes constant missing from pool".to_string());
			return;
		};
		self.ins(Instruction::I32Const(types::TAG_BYTES));
		self.ins(Instruction::I32Const(off as i32));
		self.ins(Instruction::I32Const(len as i32));
		self.ins(Instruction::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		self.ins(Instruction::StructNew(types::T_STR));
	}

	fn push_nothing(&mut self) {
		self.ins(Instruction::I32Const(types::TAG_NOTHING));
		self.ins(Instruction::StructNew(types::T_VALUE));
	}

	/// Push a `$str` value for a string constant (from the shared data segment).
	fn string_const(&mut self, s: &str) {
		let Some(&(off, len)) = self.strpool.at.get(s) else {
			self
				.diags
				.push(format!("string constant {s:?} missing from pool"));
			return;
		};
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::I32Const(off as i32));
		self.ins(Instruction::I32Const(len as i32));
		self.ins(Instruction::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		self.ins(Instruction::StructNew(types::T_STR));
	}

	/// Push the `$bytes` backing array of a string-typed atom.
	fn str_bytes(&mut self, a: &Atom) {
		self.atom(a);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_STR,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_STR,
			field_index: 1,
		});
	}

	/// Push a `$valarray` built from the given atoms (boxed).
	fn elems_array(&mut self, elems: &[Atom]) {
		for a in elems {
			self.atom(a);
		}
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: elems.len() as u32,
		});
	}

	/// `MakeList`: an element-only list builds a `$valarray` directly. A spread
	/// (`[a, ...xs, b]`) builds each segment's array — a fixed array for each run
	/// of plain elements, a list's element array for each `...spread` — and folds
	/// them with `__arrconcat`, wrapping the result in a `$list`.
	fn make_list(&mut self, items: &[ir::ListItem]) {
		use ir::ListItem;
		if !items.iter().any(|it| matches!(it, ListItem::Spread(_))) {
			self.ins(Instruction::I32Const(types::TAG_LIST));
			self.elems_array(
				&items
					.iter()
					.map(|it| match it {
						ListItem::Elem(a) => a.clone(),
						ListItem::Spread(_) => unreachable!(),
					})
					.collect::<Vec<_>>(),
			);
			self.ins(Instruction::StructNew(types::T_LIST));
			return;
		}
		let concat = self.runtime.arrconcat_fn.expect("arrconcat");
		// Group items into segments: runs of plain elements vs. single spreads.
		let mut segs: Vec<Vec<&Atom>> = Vec::new();
		let mut spread_at: Vec<bool> = Vec::new();
		for it in items {
			match it {
				ListItem::Elem(a) => {
					if spread_at.last() == Some(&false) {
						segs.last_mut().unwrap().push(a);
					} else {
						segs.push(vec![a]);
						spread_at.push(false);
					}
				}
				ListItem::Spread(a) => {
					segs.push(vec![a]);
					spread_at.push(true);
				}
			}
		}
		// Emit each segment's $valarray, folding with __arrconcat.
		for (i, (seg, &is_spread)) in segs.iter().zip(&spread_at).enumerate() {
			if is_spread {
				self.atom(seg[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
			} else {
				for a in seg {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: seg.len() as u32,
				});
			}
			if i > 0 {
				self.ins(Instruction::Call(concat));
			}
		}
		let tmp = self.fresh_local(types::valarray_ref());
		self.ins(Instruction::LocalSet(tmp));
		self.ins(Instruction::I32Const(types::TAG_LIST));
		self.ins(Instruction::LocalGet(tmp));
		self.ins(Instruction::StructNew(types::T_LIST));
	}

	fn atom_repr(&self, a: &Atom) -> Repr {
		match a {
			Atom::Var(v) => self
				.f
				.var_reprs
				.get(v.0 as usize)
				.copied()
				.unwrap_or(Repr::Boxed),
			Atom::Const(Const::Int(_)) => Repr::I64,
			Atom::Const(Const::Float(_)) => Repr::F64,
			Atom::Const(Const::Bool(_)) => Repr::I32,
			Atom::Const(_) => Repr::Boxed,
		}
	}

	fn local(&self, var: u32) -> u32 {
		self.locals[var as usize]
	}

	fn ins(&mut self, ins: Instruction<'static>) {
		self.body.push(ins);
	}
}

/// Build the structural-equality runtime helper `__eq(a, b) -> i32` (1 = equal).
/// Recursive over variants; loops over string bytes. Mirrors `vm`'s structural
/// `==`: same-typed operands (the type checker guarantees it), IEEE float compare
/// (so `nan != nan`), byte-exact strings. `self_idx` is `__eq`'s own wasm index
/// (for the variant-payload recursion). Tuples/lists/records are not yet handled
/// (they trap — a clear signal to implement them, not a silent wrong answer).
fn build_eq_fn(self_idx: u32) -> Function {
	use Instruction as I;
	// Locals past the two params: ta, tb, i, n (i32); aa, bb ($bytes); pa, pb
	// ($valarray); j, found (i32, for the order-independent dict compare).
	let locals = vec![
		ValType::I32,
		ValType::I32,
		ValType::I32,
		ValType::I32,
		types::bytes_ref(),
		types::bytes_ref(),
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
	];
	const A: u32 = 0;
	const B: u32 = 1;
	const TA: u32 = 2;
	const TB: u32 = 3;
	const I_: u32 = 4;
	const N: u32 = 5;
	const AA: u32 = 6;
	const BB: u32 = 7;
	const PA: u32 = 8;
	const PB: u32 = 9;
	const J: u32 = 10;
	const FOUND: u32 = 11;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = Vec::new();
	// ta = tag(a); tb = tag(b); if ta != tb -> 0.
	b.push(I::LocalGet(A));
	b.push(getf(types::T_VALUE, 0));
	b.push(I::LocalSet(TA));
	b.push(I::LocalGet(B));
	b.push(getf(types::T_VALUE, 0));
	b.push(I::LocalSet(TB));
	b.push(I::LocalGet(TA));
	b.push(I::LocalGet(TB));
	b.push(I::I32Ne);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	// Per-tag scalar cases, each returning.
	let scalar = |b: &mut Vec<Instruction>, tag: i32, ty: u32, eq: Instruction<'static>| {
		b.push(I::LocalGet(TA));
		b.push(I::I32Const(tag));
		b.push(I::I32Eq);
		b.push(I::If(empty));
		b.push(I::LocalGet(A));
		b.push(cast(ty));
		b.push(getf(ty, 1));
		b.push(I::LocalGet(B));
		b.push(cast(ty));
		b.push(getf(ty, 1));
		b.push(eq);
		b.push(I::Return);
		b.push(I::End);
	};
	// NOTHING -> equal.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_NOTHING));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	b.push(I::I32Const(1));
	b.push(I::Return);
	b.push(I::End);
	scalar(&mut b, types::TAG_BOOL, types::T_BOOL, I::I32Eq);
	scalar(&mut b, types::TAG_INT, types::T_INT, I::I64Eq);
	scalar(&mut b, types::TAG_FLOAT, types::T_FLOAT, I::F64Eq);
	// STR / BYTES (same `{tag, $bytes}` shape): equal lengths and equal bytes.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_STR));
	b.push(I::I32Eq);
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_BYTES));
	b.push(I::I32Eq);
	b.push(I::I32Or);
	b.push(I::If(empty));
	{
		b.push(I::LocalGet(A));
		b.push(cast(types::T_STR));
		b.push(getf(types::T_STR, 1));
		b.push(I::LocalSet(AA));
		b.push(I::LocalGet(B));
		b.push(cast(types::T_STR));
		b.push(getf(types::T_STR, 1));
		b.push(I::LocalSet(BB));
		b.push(I::LocalGet(AA));
		b.push(I::ArrayLen);
		b.push(I::LocalSet(N));
		b.push(I::LocalGet(BB));
		b.push(I::ArrayLen);
		b.push(I::LocalGet(N));
		b.push(I::I32Ne);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::I32Const(0));
		b.push(I::LocalSet(I_));
		b.push(I::Block(empty)); // $brk
		b.push(I::Loop(empty)); // $lp
		b.push(I::LocalGet(I_));
		b.push(I::LocalGet(N));
		b.push(I::I32GeS);
		b.push(I::BrIf(1)); // -> $brk
		b.push(I::LocalGet(AA));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGetU(types::T_BYTES));
		b.push(I::LocalGet(BB));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGetU(types::T_BYTES));
		b.push(I::I32Ne);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::LocalGet(I_));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(I_));
		b.push(I::Br(0)); // -> $lp
		b.push(I::End); // loop
		b.push(I::End); // block
		b.push(I::I32Const(1));
		b.push(I::Return);
	}
	b.push(I::End);
	// Element-wise array compare (recursive). Loads the `$valarray` at field
	// `field` of both `a`/`b` (cast to `sty`), checks equal lengths, then compares
	// each element via `__eq`; emits the success `return 1`.
	let cmp_array = |b: &mut Vec<Instruction>, sty: u32, field: u32| {
		b.push(I::LocalGet(A));
		b.push(cast(sty));
		b.push(getf(sty, field));
		b.push(I::LocalSet(PA));
		b.push(I::LocalGet(B));
		b.push(cast(sty));
		b.push(getf(sty, field));
		b.push(I::LocalSet(PB));
		// Lengths must match.
		b.push(I::LocalGet(PA));
		b.push(I::ArrayLen);
		b.push(I::LocalSet(N));
		b.push(I::LocalGet(PB));
		b.push(I::ArrayLen);
		b.push(I::LocalGet(N));
		b.push(I::I32Ne);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::I32Const(0));
		b.push(I::LocalSet(I_));
		b.push(I::Block(empty)); // $brk
		b.push(I::Loop(empty)); // $lp
		b.push(I::LocalGet(I_));
		b.push(I::LocalGet(N));
		b.push(I::I32GeS);
		b.push(I::BrIf(1)); // -> $brk
		b.push(I::LocalGet(PA));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(I::LocalGet(PB));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(I::Call(self_idx));
		b.push(I::I32Eqz);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::LocalGet(I_));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(I_));
		b.push(I::Br(0)); // -> $lp
		b.push(I::End); // loop
		b.push(I::End); // block
		b.push(I::I32Const(1));
		b.push(I::Return);
	};
	// VARIANT: equal tags, then equal payloads.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	b.push(I::LocalGet(A));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::LocalGet(B));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::I32Ne);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	cmp_array(&mut b, types::T_VARIANT, 3);
	b.push(I::End);
	// TUPLE / LIST: compare the element arrays. RECORD: compare the values arrays
	// (same type ⇒ same name-sorted fields, so positional value compare suffices).
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_TUPLE));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	cmp_array(&mut b, types::T_TUPLE, 1);
	b.push(I::End);
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_LIST));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	cmp_array(&mut b, types::T_LIST, 1);
	b.push(I::End);
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_RECORD));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	cmp_array(&mut b, types::T_RECORD, 2);
	b.push(I::End);
	// REF: reference identity (`ref.eq`), matching the VM's `Rc::ptr_eq` — two
	// cells are equal iff they are the same cell, regardless of contents.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_REF));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	b.push(I::LocalGet(A));
	b.push(I::LocalGet(B));
	b.push(I::RefEq);
	b.push(I::Return);
	b.push(I::End);
	// DICT: order-independent structural compare (matches the VM). Equal sizes,
	// then every entry of `a` must have a key in `b` with an equal value. Keys are
	// unique within each dict, so equal sizes make this a bijection check. Entry
	// fields are read inline: `entries[idx]` is a `$tuple`, elem 0 = key, 1 = value.
	let entry_field = |b: &mut Vec<Instruction>, arr: u32, idx: u32, field: i32| {
		b.push(I::LocalGet(arr));
		b.push(I::LocalGet(idx));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(cast(types::T_TUPLE));
		b.push(getf(types::T_TUPLE, 1));
		b.push(I::I32Const(field));
		b.push(I::ArrayGet(types::T_VALARRAY));
	};
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	// PA = a.entries; PB = b.entries; N = len(a); bail if lengths differ.
	b.push(I::LocalGet(A));
	b.push(cast(types::T_DICT));
	b.push(getf(types::T_DICT, 1));
	b.push(I::LocalSet(PA));
	b.push(I::LocalGet(B));
	b.push(cast(types::T_DICT));
	b.push(getf(types::T_DICT, 1));
	b.push(I::LocalSet(PB));
	b.push(I::LocalGet(PA));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(PB));
	b.push(I::ArrayLen);
	b.push(I::LocalGet(N));
	b.push(I::I32Ne);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty)); // $outer
	b.push(I::Loop(empty)); // $oloop
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1)); // -> $outer (done; all matched)
	b.push(I::I32Const(0));
	b.push(I::LocalSet(J));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(FOUND));
	b.push(I::Block(empty)); // $inner
	b.push(I::Loop(empty)); // $iloop
	b.push(I::LocalGet(J));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1)); // -> $inner (key absent in b)
	// if __eq(a.key[i], b.key[j]) { ... }
	entry_field(&mut b, PA, I_, 0);
	entry_field(&mut b, PB, J, 0);
	b.push(I::Call(self_idx));
	b.push(I::If(empty));
	// values must match, else the dicts differ.
	entry_field(&mut b, PA, I_, 1);
	entry_field(&mut b, PB, J, 1);
	b.push(I::Call(self_idx));
	b.push(I::I32Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::I32Const(1));
	b.push(I::LocalSet(FOUND));
	b.push(I::Br(2)); // -> $inner (key found, move to next a-entry)
	b.push(I::End); // if key-eq
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(J));
	b.push(I::Br(0)); // -> $iloop
	b.push(I::End); // $iloop
	b.push(I::End); // $inner
	// a-key absent in b -> not equal.
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0)); // -> $oloop
	b.push(I::End); // $oloop
	b.push(I::End); // $outer
	b.push(I::I32Const(1));
	b.push(I::Return);
	b.push(I::End);
	// Unhandled (closure/ctor): not structurally compared.
	b.push(I::Unreachable);
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__getfield(record, name) -> value`: linear-scan the record's
/// name-sorted `names` array, comparing each to `name` via `__eq`; return the
/// parallel `values` element on match. Traps if absent (the type checker
/// guarantees the field exists).
fn build_getfield_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const REC: u32 = 0;
	const NAME: u32 = 1;
	const NAMES: u32 = 2;
	const VALUES: u32 = 3;
	const N: u32 = 4;
	const I_: u32 = 5;
	let empty = wasm_encoder::BlockType::Empty;
	let locals = vec![
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
	];
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = vec![
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 1),
		I::LocalSet(NAMES),
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 2),
		I::LocalSet(VALUES),
		I::LocalGet(NAMES),
		I::ArrayLen,
		I::LocalSet(N),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeS,
		I::BrIf(1), // not found -> fall out (then trap)
		I::LocalGet(NAMES),
		I::LocalGet(I_),
		I::ArrayGet(types::T_VALARRAY),
		I::LocalGet(NAME),
		I::Call(eq_idx),
		I::If(empty),
		I::LocalGet(VALUES),
		I::LocalGet(I_),
		I::ArrayGet(types::T_VALARRAY),
		I::Return,
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::Unreachable,
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__list_tail(list, n) -> list`: a fresh list of the elements from index
/// `n` (the `...rest` of a list pattern). `n` is a boxed int.
fn build_list_tail_fn() -> Function {
	use Instruction as I;
	const LIST: u32 = 0;
	const NARG: u32 = 1;
	const SRC: u32 = 2;
	const DST: u32 = 3;
	const LEN: u32 = 4;
	const N: u32 = 5;
	const I_: u32 = 6;
	let empty = wasm_encoder::BlockType::Empty;
	let locals = vec![
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		ValType::I32,
	];
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = vec![
		I::LocalGet(LIST),
		cast(types::T_LIST),
		getf(types::T_LIST, 1),
		I::LocalSet(SRC),
		I::LocalGet(SRC),
		I::ArrayLen,
		I::LocalSet(LEN),
		// n = (int) NARG
		I::LocalGet(NARG),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(N),
		// dst = new valarray of (len - n)
		I::LocalGet(LEN),
		I::LocalGet(N),
		I::I32Sub,
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(DST),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		// i >= len - n -> done
		I::LocalGet(I_),
		I::LocalGet(LEN),
		I::LocalGet(N),
		I::I32Sub,
		I::I32GeS,
		I::BrIf(1),
		// dst[i] = src[n + i]
		I::LocalGet(DST),
		I::LocalGet(I_),
		I::LocalGet(SRC),
		I::LocalGet(N),
		I::LocalGet(I_),
		I::I32Add,
		I::ArrayGet(types::T_VALARRAY),
		I::ArraySet(types::T_VALARRAY),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::I32Const(types::TAG_LIST),
		I::LocalGet(DST),
		I::StructNew(types::T_LIST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__list_build(n, f) -> list`: tabulate `[f 0, f 1, ..., f (n-1)]` in
/// one pass. `arity1` is the wasm func-type index for a 1-arg closure (env-first
/// `(value, value) -> value`), used to `call_indirect` through `f`.
fn build_list_build_fn(arity1: u32) -> Function {
	use Instruction as I;
	const N: u32 = 0; // param: n (boxed int)
	const F: u32 = 1; // param: f (closure)
	const NLEN: u32 = 2;
	const BUF: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![ValType::I32, types::valarray_ref(), ValType::I32];
	let b: Vec<Instruction> = vec![
		// nlen = (int) n
		I::LocalGet(N),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(NLEN),
		// buf = new valarray(nlen)
		I::LocalGet(NLEN),
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(BUF),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(NLEN),
		I::I32GeS,
		I::BrIf(1),
		// buf[i] = f(box i)
		I::LocalGet(BUF),
		I::LocalGet(I_),
		I::LocalGet(F),
		cast(types::T_CLOSURE), // env
		I::I32Const(types::TAG_INT),
		I::LocalGet(I_),
		I::I64ExtendI32S,
		I::StructNew(types::T_INT), // arg = box i
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		getf(types::T_CLOSURE, 1), // fn_index
		I::CallIndirect {
			type_index: arity1,
			table_index: 0,
		},
		I::ArraySet(types::T_VALARRAY),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::I32Const(types::TAG_LIST),
		I::LocalGet(BUF),
		I::StructNew(types::T_LIST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__list_collect(n, f) -> list`: like `__list_build`, but `f` returns an
/// `option`; keep each `some`'s payload in order (detected by a non-empty variant
/// payload), then trim the over-allocated buffer to the kept count.
fn build_list_collect_fn(arity1: u32) -> Function {
	use Instruction as I;
	const N: u32 = 0; // param: n (boxed int)
	const F: u32 = 1; // param: f (closure)
	const NLEN: u32 = 2;
	const BUF: u32 = 3;
	const I_: u32 = 4;
	const W: u32 = 5; // write cursor (kept count)
	const R: u32 = 6; // f's result (an option variant)
	const OUT: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![
		ValType::I32,          // NLEN
		types::valarray_ref(), // BUF
		ValType::I32,          // I_
		ValType::I32,          // W
		types::value_ref(),    // R
		types::valarray_ref(), // OUT
	];
	let b: Vec<Instruction> = vec![
		I::LocalGet(N),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(NLEN),
		I::LocalGet(NLEN),
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(BUF),
		I::I32Const(0),
		I::LocalSet(I_),
		I::I32Const(0),
		I::LocalSet(W),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(NLEN),
		I::I32GeS,
		I::BrIf(1),
		// r = f(box i)
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		I::I32Const(types::TAG_INT),
		I::LocalGet(I_),
		I::I64ExtendI32S,
		I::StructNew(types::T_INT),
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		getf(types::T_CLOSURE, 1),
		I::CallIndirect {
			type_index: arity1,
			table_index: 0,
		},
		I::LocalSet(R),
		// if r's payload is non-empty (some): buf[w] = payload[0]; w += 1
		I::LocalGet(R),
		cast(types::T_VARIANT),
		getf(types::T_VARIANT, 3),
		I::ArrayLen,
		I::If(empty),
		I::LocalGet(BUF),
		I::LocalGet(W),
		I::LocalGet(R),
		cast(types::T_VARIANT),
		getf(types::T_VARIANT, 3),
		I::I32Const(0),
		I::ArrayGet(types::T_VALARRAY),
		I::ArraySet(types::T_VALARRAY),
		I::LocalGet(W),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(W),
		I::End, // if
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		// out = new valarray(w); out[0..w] = buf[0..w]
		I::LocalGet(W),
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(OUT),
		I::LocalGet(OUT),
		I::I32Const(0),
		I::LocalGet(BUF),
		I::I32Const(0),
		I::LocalGet(W),
		I::ArrayCopy {
			array_type_index_dst: types::T_VALARRAY,
			array_type_index_src: types::T_VALARRAY,
		},
		I::I32Const(types::TAG_LIST),
		I::LocalGet(OUT),
		I::StructNew(types::T_LIST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__bytes_build(n, f) -> bytes`: tabulate an `n`-byte sequence, calling
/// `f` per index and storing its int result (truncated to 8 bits by the packed
/// `$bytes` array). `arity1` is the 1-arg closure func-type index.
fn build_bytes_build_fn(arity1: u32) -> Function {
	use Instruction as I;
	const N: u32 = 0; // param: n (boxed int)
	const F: u32 = 1; // param: f (closure)
	const NLEN: u32 = 2;
	const BUF: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![ValType::I32, types::bytes_ref(), ValType::I32];
	let b: Vec<Instruction> = vec![
		I::LocalGet(N),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(NLEN),
		I::LocalGet(NLEN),
		I::ArrayNewDefault(types::T_BYTES),
		I::LocalSet(BUF),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(NLEN),
		I::I32GeS,
		I::BrIf(1),
		// buf[i] = (i32) f(box i)
		I::LocalGet(BUF),
		I::LocalGet(I_),
		I::LocalGet(F),
		cast(types::T_CLOSURE), // env
		I::I32Const(types::TAG_INT),
		I::LocalGet(I_),
		I::I64ExtendI32S,
		I::StructNew(types::T_INT), // arg = box i
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		getf(types::T_CLOSURE, 1), // fn_index
		I::CallIndirect {
			type_index: arity1,
			table_index: 0,
		},
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64, // unbox result to i32 (array.set packs to i8)
		I::ArraySet(types::T_BYTES),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::I32Const(types::TAG_BYTES),
		I::LocalGet(BUF),
		I::StructNew(types::T_STR),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

// ---------------------------------------------------------------------------
// Dict helpers. A `$dict` is `{tag, $valarray entries}` where each entry is a
// `$tuple (key, value)`. We linear-scan with `__eq` on keys — the VM's hash
// buckets are a pure accelerator, so insertion-order + structural key equality
// fully determine observable behavior. insert/lookup/remove DROP the hash
// method-dict the `where (hash k)` constraint passes (handled at the call site).
// ---------------------------------------------------------------------------

/// Emit the `$valarray` of the dict in `D` (param 0), i.e. `D.entries`.
fn dict_entries_of(b: &mut Vec<Instruction>, d: u32) {
	use Instruction as I;
	b.push(I::LocalGet(d));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_DICT)));
	b.push(I::StructGet {
		struct_type_index: types::T_DICT,
		field_index: 1,
	});
}

/// Emit `entries[idx_local].elems[field]` — the key (field 0) or value (1) of
/// the `$tuple` entry at `idx_local` in the `$valarray` held in `arr_local`.
fn dict_entry_field(b: &mut Vec<Instruction>, arr_local: u32, idx_local: u32, field: i32) {
	use Instruction as I;
	b.push(I::LocalGet(arr_local));
	b.push(I::LocalGet(idx_local));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_TUPLE)));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::I32Const(field));
	b.push(I::ArrayGet(types::T_VALARRAY));
}

/// Build `__dict_insert(dict, key, value) -> dict`: scan for `key` (via `__eq`);
/// replace its entry if present, else append. Returns a fresh `$dict`.
fn build_dict_insert_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const K: u32 = 1;
	const V: u32 = 2;
	const ENTRIES: u32 = 3;
	const N: u32 = 4;
	const I_: u32 = 5;
	const FOUND: u32 = 6;
	const NEW: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	// `NEW[at] = tuple(K, V)`.
	let store_kv = |b: &mut Vec<Instruction>, at: &dyn Fn(&mut Vec<Instruction>)| {
		b.push(I::LocalGet(NEW));
		at(b);
		b.push(I::I32Const(types::TAG_TUPLE));
		b.push(I::LocalGet(K));
		b.push(I::LocalGet(V));
		b.push(I::ArrayNewFixed {
			array_type_index: va,
			array_size: 2,
		});
		b.push(I::StructNew(types::T_TUPLE));
		b.push(I::ArraySet(va));
	};
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(-1));
	b.push(I::LocalSet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	// Keys are unique, so the last (==only) match is the entry to replace.
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalGet(K));
	b.push(I::Call(eq_idx));
	b.push(I::If(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalSet(FOUND));
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	// Pre-init NEW (a non-null local) so it is definitely-assigned on every path
	// — the validator does not merge assignments made only inside if/else arms.
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalSet(NEW));
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::I32GeS);
	b.push(I::If(empty));
	{
		// Replace: NEW = copy of ENTRIES; NEW[FOUND] = (K, V).
		b.push(I::LocalGet(N));
		b.push(I::ArrayNewDefault(va));
		b.push(I::LocalSet(NEW));
		b.push(I::LocalGet(NEW));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(ENTRIES));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(N));
		b.push(I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		});
		store_kv(&mut b, &|b| b.push(I::LocalGet(FOUND)));
	}
	b.push(I::Else);
	{
		// Append: NEW = copy of ENTRIES grown by one; NEW[N] = (K, V).
		b.push(I::LocalGet(N));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::ArrayNewDefault(va));
		b.push(I::LocalSet(NEW));
		b.push(I::LocalGet(NEW));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(ENTRIES));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(N));
		b.push(I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		});
		store_kv(&mut b, &|b| b.push(I::LocalGet(N)));
	}
	b.push(I::End);
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(NEW));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![types::valarray_ref(), ValType::I32, ValType::I32, ValType::I32, types::valarray_ref()];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_lookup(dict, key) -> option value`: linear scan via `__eq`.
fn build_dict_lookup_fn(eq_idx: u32, opt: OptionLits) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const K: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	// Push a fresh `$str` for an interned data-segment literal.
	let str_lit = |b: &mut Vec<Instruction>, (off, len): (u32, u32)| {
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::I32Const(off as i32));
		b.push(I::I32Const(len as i32));
		b.push(I::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		b.push(I::StructNew(types::T_STR));
	};
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalGet(K));
	b.push(I::Call(eq_idx));
	b.push(I::If(empty));
	// return some(value).
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Const(opt.some_tag as i32));
	str_lit(&mut b, opt.some_name);
	dict_entry_field(&mut b, ENTRIES, I_, 1);
	b.push(I::ArrayNewFixed {
		array_type_index: types::T_VALARRAY,
		array_size: 1,
	});
	b.push(I::StructNew(types::T_VARIANT));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	// none.
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Const(opt.none_tag as i32));
	str_lit(&mut b, opt.none_name);
	b.push(I::ArrayNewFixed {
		array_type_index: types::T_VALARRAY,
		array_size: 0,
	});
	b.push(I::StructNew(types::T_VARIANT));
	let locals = vec![types::valarray_ref(), ValType::I32, ValType::I32];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_remove(dict, key) -> dict`: drop the matching entry (renumbered
/// dense). Returns the original dict unchanged when the key is absent.
fn build_dict_remove_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const K: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const FOUND: u32 = 5;
	const NEW: u32 = 6;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(-1));
	b.push(I::LocalSet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalGet(K));
	b.push(I::Call(eq_idx));
	b.push(I::If(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalSet(FOUND));
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	// Absent: hand back the original dict.
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::I32LtS);
	b.push(I::If(empty));
	b.push(I::LocalGet(D));
	b.push(I::Return);
	b.push(I::End);
	// NEW = array(N-1); copy [0..FOUND) then (FOUND..N) shifted down by one.
	b.push(I::LocalGet(N));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(NEW));
	b.push(I::LocalGet(NEW));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(FOUND));
	b.push(I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	});
	b.push(I::LocalGet(NEW));
	b.push(I::LocalGet(FOUND));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalGet(N));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Sub);
	b.push(I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	});
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(NEW));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![types::valarray_ref(), ValType::I32, ValType::I32, ValType::I32, types::valarray_ref()];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_map(dict, f) -> dict`: `f` over each value, keys preserved.
fn build_dict_map_fn(arity1: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const F: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const NEW: u32 = 5;
	const K: u32 = 6;
	const NV: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(NEW));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalSet(K));
	// NV = f(value): env = f, arg = value, call_indirect.
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	dict_entry_field(&mut b, ENTRIES, I_, 1);
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	b.push(I::StructGet {
		struct_type_index: types::T_CLOSURE,
		field_index: 1,
	});
	b.push(I::CallIndirect {
		type_index: arity1,
		table_index: 0,
	});
	b.push(I::LocalSet(NV));
	// NEW[i] = (K, NV).
	b.push(I::LocalGet(NEW));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(types::TAG_TUPLE));
	b.push(I::LocalGet(K));
	b.push(I::LocalGet(NV));
	b.push(I::ArrayNewFixed {
		array_type_index: va,
		array_size: 2,
	});
	b.push(I::StructNew(types::T_TUPLE));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(NEW));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		types::valarray_ref(),
		types::value_ref(),
		types::value_ref(),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_filter(dict, f) -> dict`: keep entries where `f key value` is
/// true (the entry tuple is reused verbatim).
fn build_dict_filter_fn(arity2: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const F: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const TMP: u32 = 5;
	const W: u32 = 6;
	const K: u32 = 7;
	const V: u32 = 8;
	const OUT: u32 = 9;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(TMP));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(W));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalSet(K));
	dict_entry_field(&mut b, ENTRIES, I_, 1);
	b.push(I::LocalSet(V));
	// keep = f(k, v): env = f, args k v, call_indirect; unbox the $bool.
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	b.push(I::LocalGet(K));
	b.push(I::LocalGet(V));
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	b.push(I::StructGet {
		struct_type_index: types::T_CLOSURE,
		field_index: 1,
	});
	b.push(I::CallIndirect {
		type_index: arity2,
		table_index: 0,
	});
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_BOOL)));
	b.push(I::StructGet {
		struct_type_index: types::T_BOOL,
		field_index: 1,
	});
	b.push(I::If(empty));
	// TMP[W] = entry; W += 1.
	b.push(I::LocalGet(TMP));
	b.push(I::LocalGet(W));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(W));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(W));
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	// OUT = array(W); copy TMP[0..W].
	b.push(I::LocalGet(W));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(OUT));
	b.push(I::LocalGet(OUT));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(TMP));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(W));
	b.push(I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	});
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(OUT));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		types::valarray_ref(),
		ValType::I32,
		types::value_ref(),
		types::value_ref(),
		types::valarray_ref(),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__arrconcat(a, b) -> valarray`: a fresh array holding `a` then `b`.
fn build_arrconcat_fn() -> Function {
	use Instruction as I;
	const A: u32 = 0;
	const B: u32 = 1;
	const LA: u32 = 2;
	const LB: u32 = 3;
	const DST: u32 = 4;
	let va = types::T_VALARRAY;
	let copy = I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	};
	let locals = vec![ValType::I32, ValType::I32, types::valarray_ref()];
	let b: Vec<Instruction> = vec![
		I::LocalGet(A),
		I::ArrayLen,
		I::LocalSet(LA),
		I::LocalGet(B),
		I::ArrayLen,
		I::LocalSet(LB),
		// dst = new valarray(la + lb)
		I::LocalGet(LA),
		I::LocalGet(LB),
		I::I32Add,
		I::ArrayNewDefault(va),
		I::LocalSet(DST),
		// dst[0..la] = a
		I::LocalGet(DST),
		I::I32Const(0),
		I::LocalGet(A),
		I::I32Const(0),
		I::LocalGet(LA),
		copy.clone(),
		// dst[la..la+lb] = b
		I::LocalGet(DST),
		I::LocalGet(LA),
		I::LocalGet(B),
		I::I32Const(0),
		I::LocalGet(LB),
		copy,
		I::LocalGet(DST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__bytesconcat(a, b) -> bytes`: a fresh byte array holding `a` then `b`.
fn build_bytesconcat_fn() -> Function {
	use Instruction as I;
	const A: u32 = 0;
	const B: u32 = 1;
	const LA: u32 = 2;
	const LB: u32 = 3;
	const DST: u32 = 4;
	let bv = types::T_BYTES;
	let copy = I::ArrayCopy {
		array_type_index_dst: bv,
		array_type_index_src: bv,
	};
	let locals = vec![ValType::I32, ValType::I32, types::bytes_ref()];
	let b: Vec<Instruction> = vec![
		I::LocalGet(A),
		I::ArrayLen,
		I::LocalSet(LA),
		I::LocalGet(B),
		I::ArrayLen,
		I::LocalSet(LB),
		I::LocalGet(LA),
		I::LocalGet(LB),
		I::I32Add,
		I::ArrayNewDefault(bv),
		I::LocalSet(DST),
		I::LocalGet(DST),
		I::I32Const(0),
		I::LocalGet(A),
		I::I32Const(0),
		I::LocalGet(LA),
		copy.clone(),
		I::LocalGet(DST),
		I::LocalGet(LA),
		I::LocalGet(B),
		I::I32Const(0),
		I::LocalGet(LB),
		copy,
		I::LocalGet(DST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// FNV-1a 64-bit offset basis / prime — the constants `vm::wire` mixes with, so
/// the wasm fingerprint matches the VM's byte-for-byte.
const WIRE_FNV_OFFSET: i64 = 0xcbf2_9ce4_8422_2325u64 as i64;
const WIRE_FNV_PRIME: i64 = 0x0000_0100_0000_01b3;

/// Build `__wire_mix_len(i64 h, i64 n) -> i64`: fold `mix_byte` over `n`'s 8
/// little-endian bytes (mirrors `vm::wire::mix_len`, where lengths are `u64` LE).
fn build_wire_mix_len_fn() -> Function {
	use Instruction as I;
	const H: u32 = 0;
	const N: u32 = 1;
	const I_: u32 = 2;
	let empty = wasm_encoder::BlockType::Empty;
	let mut b: Vec<Instruction> = vec![
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::I32Const(8),
		I::I32GeU,
		I::BrIf(1),
		// h = (h ^ ((n >> (i*8)) & 0xff)) * PRIME
		I::LocalGet(H),
		I::LocalGet(N),
		I::LocalGet(I_),
		I::I32Const(8),
		I::I32Mul,
		I::I64ExtendI32U,
		I::I64ShrU,
		I::I64Const(0xff),
		I::I64And,
		I::I64Xor,
		I::I64Const(WIRE_FNV_PRIME),
		I::I64Mul,
		I::LocalSet(H),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::LocalGet(H),
	];
	let mut f = Function::new_with_locals_types(vec![ValType::I32]);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__wire_mix_str(i64 h, ref $value str) -> i64`: mix the string's byte
/// length (via `mix_len`) then each of its bytes (mirrors `vm::wire::mix_str`).
fn build_wire_mix_str_fn(mix_len: u32) -> Function {
	use Instruction as I;
	const H: u32 = 0;
	const S: u32 = 1;
	const BYTES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = vec![
		// bytes = (cast $str s).field1
		I::LocalGet(S),
		I::RefCastNonNull(HeapType::Concrete(types::T_STR)),
		getf(types::T_STR, 1),
		I::LocalSet(BYTES),
		// n = array.len bytes; h = mix_len(h, n)
		I::LocalGet(BYTES),
		I::ArrayLen,
		I::LocalSet(N),
		I::LocalGet(H),
		I::LocalGet(N),
		I::I64ExtendI32U,
		I::Call(mix_len),
		I::LocalSet(H),
		// for i in 0..n: h = (h ^ byte) * PRIME
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeU,
		I::BrIf(1),
		I::LocalGet(H),
		I::LocalGet(BYTES),
		I::LocalGet(I_),
		I::ArrayGetU(types::T_BYTES),
		I::I64ExtendI32U,
		I::I64Xor,
		I::I64Const(WIRE_FNV_PRIME),
		I::I64Mul,
		I::LocalSet(H),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::LocalGet(H),
	];
	let mut f = Function::new_with_locals_types(vec![types::bytes_ref(), ValType::I32, ValType::I32]);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__wire_fp(i64 h, ref $value schema) -> i64`: the recursive structural
/// fingerprint over a `wire-schema` value tree (mirrors `vm::wire::mix_schema`).
/// Dispatches on the schema node's `vtag`; each arm leads with a distinct kind
/// byte (1..13) so structurally-different schemas can't alias. `self_idx` is this
/// function's own wasm index (for recursion).
fn build_wire_fp_fn(self_idx: u32, mix_str: u32, mix_len: u32, w: WireTags) -> Function {
	use Instruction as I;
	const H: u32 = 0;
	const SCHEMA: u32 = 1;
	const VTAG: u32 = 2;
	const PAYLOAD: u32 = 3;
	const ELEMS: u32 = 4;
	const I_: u32 = 5;
	const N: u32 = 6;
	const PE: u32 = 7;
	const FIELDS: u32 = 8;
	const M: u32 = 9;
	const J: u32 = 10;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let mut b: Vec<Instruction> = Vec::new();
	// vtag = schema.variant_tag; payload = schema.payload.
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::LocalSet(VTAG));
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 3));
	b.push(I::LocalSet(PAYLOAD));
	// h = (h ^ kind) * PRIME, written back to H.
	let mix_byte = |b: &mut Vec<Instruction>, kind: i64| {
		b.push(I::LocalGet(H));
		b.push(I::I64Const(kind));
		b.push(I::I64Xor);
		b.push(I::I64Const(WIRE_FNV_PRIME));
		b.push(I::I64Mul);
		b.push(I::LocalSet(H));
	};
	// Push payload[idx] (a `$value`).
	let payload_elem = |b: &mut Vec<Instruction>, idx: i32| {
		b.push(I::LocalGet(PAYLOAD));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
	};
	// ELEMS = list-elems of payload[idx] (cast to `$list`, field 1).
	let elems_of = |b: &mut Vec<Instruction>, idx: i32, dst: u32| {
		b.push(I::LocalGet(PAYLOAD));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
		b.push(cast(types::T_LIST));
		b.push(getf(types::T_LIST, 1));
		b.push(I::LocalSet(dst));
	};
	// Scalar arm: `if vtag == t { mix_byte(kind); return h }`.
	let scalar = |b: &mut Vec<Instruction>, t: u32, kind: i64| {
		b.push(I::LocalGet(VTAG));
		b.push(I::I32Const(t as i32));
		b.push(I::I32Eq);
		b.push(I::If(empty));
		b.push(I::LocalGet(H));
		b.push(I::I64Const(kind));
		b.push(I::I64Xor);
		b.push(I::I64Const(WIRE_FNV_PRIME));
		b.push(I::I64Mul);
		b.push(I::Return);
		b.push(I::End);
	};
	scalar(&mut b, w.s_int, 1);
	scalar(&mut b, w.s_float, 2);
	scalar(&mut b, w.s_bool, 3);
	scalar(&mut b, w.s_string, 4);
	scalar(&mut b, w.s_bytes, 5);
	scalar(&mut b, w.s_duration, 6);
	scalar(&mut b, w.s_nothing, 7);
	// Open the `if vtag == t {` for a compound arm.
	let open = |b: &mut Vec<Instruction>, t: u32| {
		b.push(I::LocalGet(VTAG));
		b.push(I::I32Const(t as i32));
		b.push(I::I32Eq);
		b.push(I::If(empty));
	};
	// s-list: wire_fp(mix_byte(h, 8), inner=payload[0]).
	open(&mut b, w.s_list);
	mix_byte(&mut b, 8);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(self_idx));
	b.push(I::Return);
	b.push(I::End);
	// s-dict: wire_fp(wire_fp(mix_byte(h, 12), k=payload[0]), v=payload[1]).
	open(&mut b, w.s_dict);
	mix_byte(&mut b, 12);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(self_idx));
	b.push(I::LocalSet(H));
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 1);
	b.push(I::Call(self_idx));
	b.push(I::Return);
	b.push(I::End);
	// s-enum-ref: mix_str(mix_byte(h, 13), qualified=payload[0]).
	open(&mut b, w.s_enum_ref);
	mix_byte(&mut b, 13);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(mix_str));
	b.push(I::Return);
	b.push(I::End);
	// Fold `wire_fp` over the `$valarray` in local `arr`, length `N`, using loop
	// index `idx`; accumulates into `H`.
	let fold_fp = |b: &mut Vec<Instruction>, arr: u32, idx: u32| {
		b.push(I::I32Const(0));
		b.push(I::LocalSet(idx));
		b.push(I::Block(empty));
		b.push(I::Loop(empty));
		b.push(I::LocalGet(idx));
		b.push(I::LocalGet(N));
		b.push(I::I32GeU);
		b.push(I::BrIf(1));
		b.push(I::LocalGet(H));
		b.push(I::LocalGet(arr));
		b.push(I::LocalGet(idx));
		b.push(I::ArrayGet(va));
		b.push(I::Call(self_idx));
		b.push(I::LocalSet(H));
		b.push(I::LocalGet(idx));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(idx));
		b.push(I::Br(0));
		b.push(I::End); // loop
		b.push(I::End); // block
	};
	// h = mix_len(h, (i64) local N).
	let mix_len_n = |b: &mut Vec<Instruction>| {
		b.push(I::LocalGet(H));
		b.push(I::LocalGet(N));
		b.push(I::I64ExtendI32U);
		b.push(I::Call(mix_len));
		b.push(I::LocalSet(H));
	};
	// s-tuple: mix_len(mix_byte(h, 9), elems.len()); fold wire_fp over elems.
	open(&mut b, w.s_tuple);
	mix_byte(&mut b, 9);
	elems_of(&mut b, 0, ELEMS);
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	mix_len_n(&mut b);
	fold_fp(&mut b, ELEMS, I_);
	b.push(I::LocalGet(H));
	b.push(I::Return);
	b.push(I::End);
	// s-record: mix_len(mix_byte(h, 10), fields.len()); each field is a
	// `$tuple (name, schema)` — mix_str the name, recurse on the schema.
	open(&mut b, w.s_record);
	mix_byte(&mut b, 10);
	elems_of(&mut b, 0, ELEMS);
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	mix_len_n(&mut b);
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// PE = (cast $tuple elems[i]).field1
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(PE));
	// h = mix_str(h, PE[0])
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::Call(mix_str));
	b.push(I::LocalSet(H));
	// h = wire_fp(h, PE[1])
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalSet(H));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::LocalGet(H));
	b.push(I::Return);
	b.push(I::End);
	// s-enum: mix_len(mix_str(mix_byte(h, 11), qualified), variants.len()); each
	// variant is a `$tuple (name, list-of-field-schemas)`.
	open(&mut b, w.s_enum);
	mix_byte(&mut b, 11);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(mix_str));
	b.push(I::LocalSet(H));
	// ELEMS = variants list (payload[1] is a `$list`).
	b.push(I::LocalGet(PAYLOAD));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(ELEMS));
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	mix_len_n(&mut b);
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty)); // over variants
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// PE = (cast $tuple variants[i]).field1  (name, field-list)
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(PE));
	// h = mix_str(h, PE[0])  (variant name)
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::Call(mix_str));
	b.push(I::LocalSet(H));
	// FIELDS = (cast $list PE[1]).field1; M = len; h = mix_len(h, M)
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(FIELDS));
	b.push(I::LocalGet(FIELDS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(M));
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(M));
	b.push(I::I64ExtendI32U);
	b.push(I::Call(mix_len));
	b.push(I::LocalSet(H));
	// for j in 0..M: h = wire_fp(h, FIELDS[j])
	b.push(I::I32Const(0));
	b.push(I::LocalSet(J));
	b.push(I::Block(empty));
	b.push(I::Loop(empty)); // over fields
	b.push(I::LocalGet(J));
	b.push(I::LocalGet(M));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(FIELDS));
	b.push(I::LocalGet(J));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalSet(H));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(J));
	b.push(I::Br(0));
	b.push(I::End); // inner loop
	b.push(I::End); // inner block
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // outer loop
	b.push(I::End); // outer block
	b.push(I::LocalGet(H));
	b.push(I::Return);
	b.push(I::End);
	// Fallthrough (malformed schema): return h unchanged.
	b.push(I::LocalGet(H));
	let locals = vec![
		ValType::I32,          // VTAG
		types::valarray_ref(), // PAYLOAD
		types::valarray_ref(), // ELEMS
		ValType::I32,          // I_
		ValType::I32,          // N
		types::valarray_ref(), // PE
		types::valarray_ref(), // FIELDS
		ValType::I32,          // M
		ValType::I32,          // J
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__int_str(boxed-int) -> str`: decimal formatting of an i64. Mirrors
/// `vm::Value`'s `Display` for ints (`-` sign, no leading zeros, "0" for zero).
fn build_int_str_fn() -> Function {
	use Instruction as I;
	const V: u32 = 0; // boxed $int param
	const N: u32 = 1; // i64 value
	const NEG: u32 = 2;
	const M: u32 = 3; // abs value
	const LEN: u32 = 4;
	const TOTAL: u32 = 5;
	const BUF: u32 = 6;
	const I_: u32 = 7;
	const Q: u32 = 8;
	let empty = wasm_encoder::BlockType::Empty;
	let bv = types::T_BYTES;
	let mk_str = |b: &mut Vec<Instruction>| {
		// wrap BUF in a $str and return.
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::LocalGet(BUF));
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
	};
	let locals = vec![
		ValType::I64,
		ValType::I32,
		ValType::I64,
		ValType::I32,
		ValType::I32,
		types::bytes_ref(),
		ValType::I32,
		ValType::I64,
	];
	let mut b: Vec<Instruction> = Vec::new();
	b.push(I::LocalGet(V));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_INT)));
	b.push(I::StructGet {
		struct_type_index: types::T_INT,
		field_index: 1,
	});
	b.push(I::LocalSet(N));
	// n == 0 -> "0"
	b.push(I::LocalGet(N));
	b.push(I::I64Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(1));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::I32Const(48)); // '0'
	b.push(I::ArraySet(bv));
	mk_str(&mut b);
	b.push(I::End);
	// neg = n < 0
	b.push(I::LocalGet(N));
	b.push(I::I64Const(0));
	b.push(I::I64LtS);
	b.push(I::LocalSet(NEG));
	// m = n; if neg { m = 0 - n }
	b.push(I::LocalGet(N));
	b.push(I::LocalSet(M));
	b.push(I::LocalGet(NEG));
	b.push(I::If(empty));
	b.push(I::I64Const(0));
	b.push(I::LocalGet(N));
	b.push(I::I64Sub);
	b.push(I::LocalSet(M));
	b.push(I::End);
	// count digits: len=0; q=m; do { len++; q/=10 } while q!=0
	b.push(I::I32Const(0));
	b.push(I::LocalSet(LEN));
	b.push(I::LocalGet(M));
	b.push(I::LocalSet(Q));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(LEN));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(LEN));
	b.push(I::LocalGet(Q));
	b.push(I::I64Const(10));
	b.push(I::I64DivS);
	b.push(I::LocalSet(Q));
	b.push(I::LocalGet(Q));
	b.push(I::I64Eqz);
	b.push(I::I32Eqz);
	b.push(I::BrIf(0)); // q != 0 -> loop
	b.push(I::End);
	// total = len + neg
	b.push(I::LocalGet(LEN));
	b.push(I::LocalGet(NEG));
	b.push(I::I32Add);
	b.push(I::LocalSet(TOTAL));
	b.push(I::LocalGet(TOTAL));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	// fill from end: i = total-1; q = m; do { buf[i]=48+(q%10); q/=10; i-- } while q!=0
	b.push(I::LocalGet(TOTAL));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalSet(I_));
	b.push(I::LocalGet(M));
	b.push(I::LocalSet(Q));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(BUF));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(48));
	b.push(I::LocalGet(Q));
	b.push(I::I64Const(10));
	b.push(I::I64RemS);
	b.push(I::I32WrapI64);
	b.push(I::I32Add);
	b.push(I::ArraySet(bv));
	b.push(I::LocalGet(Q));
	b.push(I::I64Const(10));
	b.push(I::I64DivS);
	b.push(I::LocalSet(Q));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalSet(I_));
	b.push(I::LocalGet(Q));
	b.push(I::I64Eqz);
	b.push(I::I32Eqz);
	b.push(I::BrIf(0)); // q != 0 -> loop
	b.push(I::End);
	// if neg { buf[0] = '-' }
	b.push(I::LocalGet(NEG));
	b.push(I::If(empty));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::I32Const(45)); // '-'
	b.push(I::ArraySet(bv));
	b.push(I::End);
	mk_str(&mut b);
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__tostring(value) -> str` — `vm::Value`'s `Display` in wasm. Scalars +
/// string (identity) + int (`__int_str`) + float (host `float_to_str`); compounds
/// (tuple/list/record/variant) are formatted recursively, folding byte arrays with
/// `__bytesconcat`. `self_idx` is `__tostring`'s own index (for the recursion).
fn build_tostring_fn(
	self_idx: u32,
	int_str: u32,
	bc: u32,
	float_to_str: u32,
	lits: ToStringLits,
) -> Function {
	use Instruction as I;
	const V: u32 = 0;
	const TA: u32 = 1;
	const ACC: u32 = 2; // $bytes accumulator
	const I_: u32 = 3;
	const N: u32 = 4;
	const ARR: u32 = 5; // $valarray (tuple/list elems, variant payload, record values)
	const NAMES: u32 = 6; // $valarray (record names)
	const BUF: u32 = 7; // $bytes (float scratch; also bytes-arm source/dst)
	const LEN: u32 = 8; // i32 (float len; also bytes-arm write position)
	const BYTE: u32 = 9; // i32 (bytes-arm current byte)
	const NIB: u32 = 10; // i32 (bytes-arm hex nibble scratch)
	let empty = wasm_encoder::BlockType::Empty;
	let i32res = wasm_encoder::BlockType::Result(ValType::I32);
	let bv = types::T_BYTES;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	// Push a `$bytes` array for a data-segment literal.
	let lit_bytes = |b: &mut Vec<Instruction>, (off, len): (u32, u32)| {
		b.push(I::I32Const(off as i32));
		b.push(I::I32Const(len as i32));
		b.push(I::ArrayNewData {
			array_type_index: bv,
			array_data_index: 0,
		});
	};
	// `ACC = __bytesconcat(ACC, <literal>)`.
	let cat_lit = |b: &mut Vec<Instruction>, lit: (u32, u32)| {
		b.push(I::LocalGet(ACC));
		lit_bytes(b, lit);
		b.push(I::Call(bc));
		b.push(I::LocalSet(ACC));
	};
	let wrap = |b: &mut Vec<Instruction>| {
		// ACC -> $str ; return
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::LocalGet(ACC));
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
	};
	let mk_lit = |b: &mut Vec<Instruction>, lit: (u32, u32)| {
		b.push(I::I32Const(types::TAG_STR));
		lit_bytes(b, lit);
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
	};
	// `ACC = __bytesconcat(ACC, bytes-of-str(s))` where `s` (a $str value) is from
	// applying `__tostring` to element `ARR[I_]` (or a raw $str for record names).
	// Helper emitting: ACC = bytesconcat(ACC, strbytes(tostring(ARR[idx_field])))
	let cat_tostring_of = |b: &mut Vec<Instruction>, arr: u32| {
		b.push(I::LocalGet(ACC));
		b.push(I::LocalGet(arr));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(I::Call(self_idx)); // -> $str
		b.push(cast(types::T_STR));
		b.push(I::StructGet {
			struct_type_index: types::T_STR,
			field_index: 1,
		});
		b.push(I::Call(bc));
		b.push(I::LocalSet(ACC));
	};

	let locals = vec![
		ValType::I32,          // TA
		types::bytes_ref(),    // ACC
		ValType::I32,          // I_
		ValType::I32,          // N
		types::valarray_ref(), // ARR
		types::valarray_ref(), // NAMES
		types::bytes_ref(),    // BUF
		ValType::I32,          // LEN
		ValType::I32,          // BYTE
		ValType::I32,          // NIB
	];
	let mut b: Vec<Instruction> = Vec::new();
	b.push(I::LocalGet(V));
	b.push(I::StructGet {
		struct_type_index: types::T_VALUE,
		field_index: 0,
	});
	b.push(I::LocalSet(TA));

	// Scalar arm helper: if TA == tag { body }.
	let arm = |b: &mut Vec<Instruction>, tag: i32| {
		b.push(I::LocalGet(TA));
		b.push(I::I32Const(tag));
		b.push(I::I32Eq);
		b.push(I::If(empty));
	};

	// STR -> identity.
	arm(&mut b, types::TAG_STR);
	b.push(I::LocalGet(V));
	b.push(I::Return);
	b.push(I::End);
	// INT -> __int_str.
	arm(&mut b, types::TAG_INT);
	b.push(I::LocalGet(V));
	b.push(I::Call(int_str));
	b.push(I::Return);
	b.push(I::End);
	// NOTHING -> "()".
	arm(&mut b, types::TAG_NOTHING);
	mk_lit(&mut b, lits.unit);
	b.push(I::End);
	// BOOL -> "true"/"false".
	arm(&mut b, types::TAG_BOOL);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_BOOL));
	b.push(I::StructGet {
		struct_type_index: types::T_BOOL,
		field_index: 1,
	});
	b.push(I::If(empty));
	mk_lit(&mut b, lits.tru);
	b.push(I::Else);
	mk_lit(&mut b, lits.fals);
	b.push(I::End);
	b.push(I::End);
	// FLOAT -> host float_to_str into a scratch $bytes, trim to length.
	arm(&mut b, types::TAG_FLOAT);
	b.push(I::I32Const(32));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_FLOAT));
	b.push(I::StructGet {
		struct_type_index: types::T_FLOAT,
		field_index: 1,
	});
	b.push(I::LocalGet(BUF));
	b.push(I::Call(float_to_str)); // (f64, buf) -> len
	b.push(I::LocalSet(LEN));
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(ACC));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayCopy {
		array_type_index_dst: bv,
		array_type_index_src: bv,
	});
	wrap(&mut b);
	b.push(I::End);

	// BYTES -> single-quoted literal form: printable ASCII inline, `'` and
	// `\` backslash-escaped, everything else as `\xNN` (lowercase). Matches
	// `Value::Display` so wasm `to-string` agrees with the VM. Writes into a
	// worst-case (4 bytes/input + 2 quotes) buffer, then trims — no concat.
	// BUF=source/dst, ACC=output buffer, N=source len, LEN=write position.
	// Append the constant byte `code` to ACC[LEN], then bump LEN.
	let put = |b: &mut Vec<Instruction>, code: i32| {
		b.push(I::LocalGet(ACC));
		b.push(I::LocalGet(LEN));
		b.push(I::I32Const(code));
		b.push(I::ArraySet(bv));
		b.push(I::LocalGet(LEN));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(LEN));
	};
	// Append one lowercase hex digit for the nibble of BYTE at `shift`.
	let put_hex = |b: &mut Vec<Instruction>, shift: i32| {
		b.push(I::LocalGet(BYTE));
		if shift != 0 {
			b.push(I::I32Const(shift));
			b.push(I::I32ShrU);
		}
		b.push(I::I32Const(0xf));
		b.push(I::I32And);
		b.push(I::LocalSet(NIB));
		b.push(I::LocalGet(ACC));
		b.push(I::LocalGet(LEN));
		// digit = NIB < 10 ? '0'+NIB : 'a'-10+NIB  (0x61-10 = 0x57)
		b.push(I::LocalGet(NIB));
		b.push(I::I32Const(10));
		b.push(I::I32LtS);
		b.push(I::If(i32res));
		b.push(I::LocalGet(NIB));
		b.push(I::I32Const(0x30));
		b.push(I::I32Add);
		b.push(I::Else);
		b.push(I::LocalGet(NIB));
		b.push(I::I32Const(0x57));
		b.push(I::I32Add);
		b.push(I::End);
		b.push(I::ArraySet(bv));
		b.push(I::LocalGet(LEN));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(LEN));
	};
	arm(&mut b, types::TAG_BYTES);
	// BUF = source bytes; N = its length.
	b.push(I::LocalGet(V));
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(BUF));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	// ACC = new $bytes[N*4 + 2]; LEN (write pos) = 0.
	b.push(I::LocalGet(N));
	b.push(I::I32Const(4));
	b.push(I::I32Mul);
	b.push(I::I32Const(2));
	b.push(I::I32Add);
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(ACC));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(LEN));
	put(&mut b, 0x27); // opening '
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	// BYTE = source[I_] (unsigned).
	b.push(I::LocalGet(BUF));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGetU(bv));
	b.push(I::LocalSet(BYTE));
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x5c));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	put(&mut b, 0x5c);
	put(&mut b, 0x5c);
	b.push(I::Else);
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x27));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	put(&mut b, 0x5c);
	put(&mut b, 0x27);
	b.push(I::Else);
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x20));
	b.push(I::I32GeS);
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x7e));
	b.push(I::I32LeS);
	b.push(I::I32And);
	b.push(I::If(empty));
	// printable: copy the byte verbatim.
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(LEN));
	b.push(I::LocalGet(BYTE));
	b.push(I::ArraySet(bv));
	b.push(I::LocalGet(LEN));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(LEN));
	b.push(I::Else);
	put(&mut b, 0x5c); // '\'
	put(&mut b, 0x78); // 'x'
	put_hex(&mut b, 4);
	put_hex(&mut b, 0);
	b.push(I::End);
	b.push(I::End);
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	put(&mut b, 0x27); // closing '
										// Trim ACC[0..LEN] into a tight $bytes (BUF), then wrap as $str.
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(ACC));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayCopy {
		array_type_index_dst: bv,
		array_type_index_src: bv,
	});
	b.push(I::LocalGet(BUF));
	b.push(I::LocalSet(ACC));
	wrap(&mut b);
	b.push(I::End);

	// Element loop shared by TUPLE/LIST/RECORD: iterate ARR[0..N] appending
	// `__tostring(elem)` with `, ` separators. `pre`/`post` wrap the open/close.
	let elems_loop =
		|b: &mut Vec<Instruction>, arr: u32, open: (u32, u32), close: (u32, u32), record: bool| {
			// ACC = open
			lit_bytes(b, open);
			b.push(I::LocalSet(ACC));
			b.push(I::LocalGet(arr));
			b.push(I::ArrayLen);
			b.push(I::LocalSet(N));
			b.push(I::I32Const(0));
			b.push(I::LocalSet(I_));
			b.push(I::Block(empty));
			b.push(I::Loop(empty));
			b.push(I::LocalGet(I_));
			b.push(I::LocalGet(N));
			b.push(I::I32GeS);
			b.push(I::BrIf(1)); // -> end
											 // separator before all but the first
			b.push(I::LocalGet(I_));
			b.push(I::I32Const(0));
			b.push(I::I32GtS);
			b.push(I::If(empty));
			cat_lit(b, lits.comma_sp);
			b.push(I::End);
			if record {
				// "name: value": NAMES[i] is a raw $str; values in ARR.
				b.push(I::LocalGet(ACC));
				b.push(I::LocalGet(NAMES));
				b.push(I::LocalGet(I_));
				b.push(I::ArrayGet(types::T_VALARRAY));
				b.push(cast(types::T_STR));
				b.push(I::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				b.push(I::Call(bc));
				b.push(I::LocalSet(ACC));
				cat_lit(b, lits.colon_sp);
			}
			cat_tostring_of(b, arr);
			b.push(I::LocalGet(I_));
			b.push(I::I32Const(1));
			b.push(I::I32Add);
			b.push(I::LocalSet(I_));
			b.push(I::Br(0));
			b.push(I::End); // loop
			b.push(I::End); // block
			cat_lit(b, close);
			wrap(b);
		};

	// TUPLE -> "(e, ...)".
	arm(&mut b, types::TAG_TUPLE);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_TUPLE));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::LocalSet(ARR));
	elems_loop(&mut b, ARR, lits.lparen, lits.rparen, false);
	b.push(I::End);
	// LIST -> "[e, ...]".
	arm(&mut b, types::TAG_LIST);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_LIST));
	b.push(I::StructGet {
		struct_type_index: types::T_LIST,
		field_index: 1,
	});
	b.push(I::LocalSet(ARR));
	elems_loop(&mut b, ARR, lits.lbrack, lits.rbrack, false);
	b.push(I::End);
	// RECORD -> "{k: v, ...}" (name-sorted; names raw, values via __tostring).
	arm(&mut b, types::TAG_RECORD);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_RECORD));
	b.push(I::StructGet {
		struct_type_index: types::T_RECORD,
		field_index: 1,
	});
	b.push(I::LocalSet(NAMES));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_RECORD));
	b.push(I::StructGet {
		struct_type_index: types::T_RECORD,
		field_index: 2,
	});
	b.push(I::LocalSet(ARR));
	elems_loop(&mut b, ARR, lits.lbrace, lits.rbrace, true);
	b.push(I::End);
	// VARIANT -> "enum.variant" then ` arg` per payload element.
	arm(&mut b, types::TAG_VARIANT);
	// ACC = bytes-of(name).
	b.push(I::LocalGet(V));
	b.push(cast(types::T_VARIANT));
	b.push(I::StructGet {
		struct_type_index: types::T_VARIANT,
		field_index: 2,
	});
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_VARIANT));
	b.push(I::StructGet {
		struct_type_index: types::T_VARIANT,
		field_index: 3,
	});
	b.push(I::LocalSet(ARR));
	b.push(I::LocalGet(ARR));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	cat_lit(&mut b, lits.space);
	cat_tostring_of(&mut b, ARR);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	wrap(&mut b);
	b.push(I::End);

	// REF -> "ref <inner>" (matches `vm::Value`'s Display).
	arm(&mut b, types::TAG_REF);
	// ACC = bytes-of "ref ".
	lit_bytes(&mut b, lits.ref_pfx);
	b.push(I::LocalSet(ACC));
	// ACC = bytesconcat(ACC, strbytes(tostring(cell))).
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_REF));
	b.push(I::StructGet {
		struct_type_index: types::T_REF,
		field_index: 1,
	});
	b.push(I::Call(self_idx)); // -> $str
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::Call(bc));
	b.push(I::LocalSet(ACC));
	wrap(&mut b);
	b.push(I::End);

	// DICT -> "{k: v, ...}" (insertion order; each entry a `$tuple`). Mirrors
	// `vm::Value`'s Dict Display.
	arm(&mut b, types::TAG_DICT);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_DICT));
	b.push(I::StructGet {
		struct_type_index: types::T_DICT,
		field_index: 1,
	});
	b.push(I::LocalSet(ARR));
	// ACC = "{"  (set, not concat — ACC is not yet initialized here).
	lit_bytes(&mut b, lits.lbrace);
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(ARR));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	// separator before all but the first
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(0));
	b.push(I::I32GtS);
	b.push(I::If(empty));
	cat_lit(&mut b, lits.comma_sp);
	b.push(I::End);
	// key: ACC ++ tostring(entry.elems[0])
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(ARR));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(cast(types::T_TUPLE));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(I::Call(self_idx)); // -> $str
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::Call(bc));
	b.push(I::LocalSet(ACC));
	cat_lit(&mut b, lits.colon_sp);
	// value: ACC ++ tostring(entry.elems[1])
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(ARR));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(cast(types::T_TUPLE));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(I::Call(self_idx)); // -> $str
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::Call(bc));
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	cat_lit(&mut b, lits.rbrace);
	wrap(&mut b);
	b.push(I::End);

	// Unreachable: every value tag is handled above.
	b.push(I::Unreachable);
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__record_update(rec, name, value) -> rec`: a copy of `rec` with the
/// field named `name` overridden. Shares `rec`'s name array; copies its values
/// and replaces the matching slot (found via `__eq` on names).
fn build_record_update_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const REC: u32 = 0;
	const NAME: u32 = 1;
	const VALUE: u32 = 2;
	const NAMES: u32 = 3;
	const VALUES: u32 = 4;
	const NEW: u32 = 5;
	const N: u32 = 6;
	const I_: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![
		types::valarray_ref(),
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
	];
	let b: Vec<Instruction> = vec![
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 1),
		I::LocalSet(NAMES),
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 2),
		I::LocalSet(VALUES),
		I::LocalGet(VALUES),
		I::ArrayLen,
		I::LocalSet(N),
		// new = copy of values
		I::LocalGet(N),
		I::ArrayNewDefault(va),
		I::LocalSet(NEW),
		I::LocalGet(NEW),
		I::I32Const(0),
		I::LocalGet(VALUES),
		I::I32Const(0),
		I::LocalGet(N),
		I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		},
		// find name; new[i] = value; stop
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeS,
		I::BrIf(1), // not found -> done
		I::LocalGet(NAMES),
		I::LocalGet(I_),
		I::ArrayGet(va),
		I::LocalGet(NAME),
		I::Call(eq_idx),
		I::If(empty),
		I::LocalGet(NEW),
		I::LocalGet(I_),
		I::LocalGet(VALUE),
		I::ArraySet(va),
		I::Br(2), // -> done
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0), // -> loop
		I::End,   // loop
		I::End,   // block
		I::I32Const(types::TAG_RECORD),
		I::LocalGet(NAMES),
		I::LocalGet(NEW),
		I::StructNew(types::T_RECORD),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// The arity of a pure-compute builtin we can emit a wasm wrapper for, or `None`
/// if unsupported (string/bytes compare, hashes, … — later milestones).
fn builtin_arity(tag: &str) -> Option<usize> {
	Some(match tag {
		"int-add" | "int-sub" | "int-mul" | "int-div" | "float-add" | "float-sub" | "float-mul"
		| "float-div" | "int-compare" | "float-compare" | "string-compare" | "bytes-compare" => 2,
		"int-negate" | "float-negate" => 1,
		// `hash` instances: wrappable so a primitive `hash` method-dict can be
		// built, but the wasm `dict` scans with `__eq` and never calls hash, so the
		// wrapper body is unreachable (see `build_builtin_wrapper`).
		"int-hash" | "float-hash" | "string-hash" | "bool-hash" | "bytes-hash" => 1,
		_ => return None,
	})
}

/// Build the wasm wrapper for a pure-compute builtin used as a first-class value
/// (e.g. a `numeric`/`ord` dict method). Env-first closure convention: `(env,
/// args…) -> value`. Unboxes args, computes, reboxes. Comparisons return an
/// `ordering` variant; `ord` carries those variants' tags + interned display
/// names (resolved in `Module::build` when a `*-compare` wrapper is reachable).
fn build_builtin_wrapper(tag: &str, ord: &OrderingLits) -> Option<Function> {
	use Instruction as I;
	let arity = builtin_arity(tag)?;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	// Unbox arg local `n` (1-based) of scalar struct `ty` (field 1).
	let unbox = |b: &mut Vec<Instruction>, n: u32, ty: u32| {
		b.push(I::LocalGet(n));
		b.push(cast(ty));
		b.push(getf(ty, 1));
	};
	// Emit `return <ordering variant>` for the given within-enum tag + display
	// name (a 4-field `$variant` with an empty payload).
	let mk_ord = |b: &mut Vec<Instruction>, vtag: u32, (off, len): (u32, u32)| {
		b.push(I::I32Const(types::TAG_VARIANT));
		b.push(I::I32Const(vtag as i32));
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::I32Const(off as i32));
		b.push(I::I32Const(len as i32));
		b.push(I::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		b.push(I::StructNew(types::T_STR));
		b.push(I::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: 0,
		});
		b.push(I::StructNew(types::T_VARIANT));
		b.push(I::Return);
	};
	let mut b: Vec<Instruction> = Vec::new();
	let mut extra_locals: Vec<ValType> = Vec::new();

	// Arithmetic: unbox both (or one), apply op, rebox. Result staged in a temp so
	// the box tag sits below it.
	let arith = |b: &mut Vec<Instruction>,
	             extra: &mut Vec<ValType>,
	             ty: u32,
	             tag_const: i32,
	             scalar: ValType,
	             op: Instruction<'static>,
	             unary: bool| {
		let tmp = (arity + 1) as u32; // first local past env+params
		extra.push(scalar);
		if unary {
			// negate: 0 - x  (int) / f64.neg (float)
			if scalar == ValType::I64 {
				b.push(I::I64Const(0));
				b.push(I::LocalGet(1));
				b.push(cast(ty));
				b.push(getf(ty, 1));
				b.push(I::I64Sub);
			} else {
				b.push(I::LocalGet(1));
				b.push(cast(ty));
				b.push(getf(ty, 1));
				b.push(I::F64Neg);
			}
		} else {
			b.push(I::LocalGet(1));
			b.push(cast(ty));
			b.push(getf(ty, 1));
			b.push(I::LocalGet(2));
			b.push(cast(ty));
			b.push(getf(ty, 1));
			b.push(op);
		}
		b.push(I::LocalSet(tmp));
		b.push(I::I32Const(tag_const));
		b.push(I::LocalGet(tmp));
		b.push(I::StructNew(ty));
	};

	match tag {
		"int-add" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64Add,
			false,
		),
		"int-sub" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64Sub,
			false,
		),
		"int-mul" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64Mul,
			false,
		),
		"int-div" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64DivS,
			false,
		),
		"int-negate" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::Nop,
			true,
		),
		"float-add" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Add,
			false,
		),
		"float-sub" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Sub,
			false,
		),
		"float-mul" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Mul,
			false,
		),
		"float-div" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Div,
			false,
		),
		"float-negate" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::Nop,
			true,
		),
		"int-compare" | "float-compare" => {
			let (ty, lt, eq) = if tag == "int-compare" {
				(types::T_INT, I::I64LtS, I::I64Eq)
			} else {
				(types::T_FLOAT, I::F64Lt, I::F64Eq)
			};
			// a < b -> lt
			unbox(&mut b, 1, ty);
			unbox(&mut b, 2, ty);
			b.push(lt);
			b.push(I::If(wasm_encoder::BlockType::Empty));
			mk_ord(&mut b, ord.lt_tag, ord.lt_name);
			b.push(I::End);
			// a == b -> eq
			unbox(&mut b, 1, ty);
			unbox(&mut b, 2, ty);
			b.push(eq);
			b.push(I::If(wasm_encoder::BlockType::Empty));
			mk_ord(&mut b, ord.eq_tag, ord.eq_name);
			b.push(I::End);
			// else gt
			mk_ord(&mut b, ord.gt_tag, ord.gt_name);
		}
		// String / bytes ordering: lexicographic byte compare (Rust `str`/`[u8]`
		// `Ord` is byte-lexicographic). Both reuse the `$str` `{tag, $bytes}` shape,
		// so the same loop serves either. Locals past env+2 args: abytes, bbytes
		// ($bytes), alen, blen, minlen, i, av, bv (i32).
		"string-compare" | "bytes-compare" => {
			extra_locals.push(types::bytes_ref()); // 3 abytes
			extra_locals.push(types::bytes_ref()); // 4 bbytes
			extra_locals.push(ValType::I32); // 5 alen
			extra_locals.push(ValType::I32); // 6 blen
			extra_locals.push(ValType::I32); // 7 minlen
			extra_locals.push(ValType::I32); // 8 i
			const ABYTES: u32 = 3;
			const BBYTES: u32 = 4;
			const ALEN: u32 = 5;
			const BLEN: u32 = 6;
			const MINLEN: u32 = 7;
			const I_: u32 = 8;
			let empty = wasm_encoder::BlockType::Empty;
			// abytes = (cast $str a).field1; bbytes likewise.
			b.push(I::LocalGet(1));
			b.push(cast(types::T_STR));
			b.push(getf(types::T_STR, 1));
			b.push(I::LocalSet(ABYTES));
			b.push(I::LocalGet(2));
			b.push(cast(types::T_STR));
			b.push(getf(types::T_STR, 1));
			b.push(I::LocalSet(BBYTES));
			// alen / blen / minlen.
			b.push(I::LocalGet(ABYTES));
			b.push(I::ArrayLen);
			b.push(I::LocalSet(ALEN));
			b.push(I::LocalGet(BBYTES));
			b.push(I::ArrayLen);
			b.push(I::LocalSet(BLEN));
			b.push(I::LocalGet(ALEN));
			b.push(I::LocalGet(BLEN));
			b.push(I::I32LtU);
			b.push(I::If(wasm_encoder::BlockType::Result(ValType::I32)));
			b.push(I::LocalGet(ALEN));
			b.push(I::Else);
			b.push(I::LocalGet(BLEN));
			b.push(I::End);
			b.push(I::LocalSet(MINLEN));
			// i = 0; scan the shared prefix.
			b.push(I::I32Const(0));
			b.push(I::LocalSet(I_));
			b.push(I::Block(empty)); // $done
			b.push(I::Loop(empty)); // $cmp
			b.push(I::LocalGet(I_));
			b.push(I::LocalGet(MINLEN));
			b.push(I::I32GeU);
			b.push(I::BrIf(1)); // -> $done
			// av < bv -> less ; av > bv -> greater (unsigned byte compare).
			b.push(I::LocalGet(ABYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::LocalGet(BBYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::I32LtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.lt_tag, ord.lt_name);
			b.push(I::End);
			b.push(I::LocalGet(ABYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::LocalGet(BBYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::I32GtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.gt_tag, ord.gt_name);
			b.push(I::End);
			b.push(I::LocalGet(I_));
			b.push(I::I32Const(1));
			b.push(I::I32Add);
			b.push(I::LocalSet(I_));
			b.push(I::Br(0)); // -> $cmp
			b.push(I::End); // loop
			b.push(I::End); // block $done
			// Prefix equal: shorter sorts first.
			b.push(I::LocalGet(ALEN));
			b.push(I::LocalGet(BLEN));
			b.push(I::I32LtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.lt_tag, ord.lt_name);
			b.push(I::End);
			b.push(I::LocalGet(ALEN));
			b.push(I::LocalGet(BLEN));
			b.push(I::I32GtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.gt_tag, ord.gt_name);
			b.push(I::End);
			mk_ord(&mut b, ord.eq_tag, ord.eq_name);
		}
		// `hash` wrappers exist only so a primitive `hash` method-dict can be
		// materialized; the wasm `dict` never calls them (it scans keys with
		// `__eq`). A trap keeps it honest if some future caller does reach one.
		"int-hash" | "float-hash" | "string-hash" | "bool-hash" | "bytes-hash" => {
			b.push(I::Unreachable);
		}
		_ => return None,
	}

	// `extra_locals` are the locals past the env+arg params (which come from the
	// function's declared type, not declared here).
	let mut f = Function::new_with_locals_types(extra_locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	Some(f)
}

/// Resolve a variant name to its within-enum tag across all enums (unique-name or
/// shared-tag assumption, as in `FnEmitter::variant_tag`).
fn variant_tag_in(enums: &EnumTable, name: &str) -> Option<u32> {
	let mut found = None;
	for vs in enums.values() {
		if let Some(i) = vs.iter().position(|(n, _)| n == name) {
			match found {
				None => found = Some(i as u32),
				Some(t) if t == i as u32 => {}
				Some(_) => return None,
			}
		}
	}
	found
}

fn repr_valtype(r: Repr) -> ValType {
	match r {
		Repr::Boxed => types::value_ref(),
		Repr::I64 => ValType::I64,
		Repr::F64 => ValType::F64,
		Repr::I32 => ValType::I32,
	}
}

fn binop_instr(op: ir::BinOp) -> Option<Instruction<'static>> {
	use ir::BinOp::*;
	Some(match op {
		AddInt => Instruction::I64Add,
		SubInt => Instruction::I64Sub,
		MulInt => Instruction::I64Mul,
		DivInt => Instruction::I64DivS,
		RemInt => Instruction::I64RemS,
		AddFloat => Instruction::F64Add,
		SubFloat => Instruction::F64Sub,
		MulFloat => Instruction::F64Mul,
		DivFloat => Instruction::F64Div,
		// f64 has no remainder opcode; RemFloat needs a runtime helper (later).
		RemFloat => return None,
		// Ordering comparisons, split by operand repr; result is i32 (bool).
		LtI64 => Instruction::I64LtS,
		LeI64 => Instruction::I64LeS,
		GtI64 => Instruction::I64GtS,
		GeI64 => Instruction::I64GeS,
		LtF64 => Instruction::F64Lt,
		LeF64 => Instruction::F64Le,
		GtF64 => Instruction::F64Gt,
		GeF64 => Instruction::F64Ge,
		// Strict logical and/or over i32 booleans (both operands evaluated).
		And => Instruction::I32And,
		Or => Instruction::I32Or,
		// Structural equality and string concat need runtime helpers (later).
		Eq | Ne | Concat => return None,
	})
}
