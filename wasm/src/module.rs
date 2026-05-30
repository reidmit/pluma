// Module assembly. `Module::build` lays out the wasm module: host imports for the
// builtins a program actually calls, one defined function per reachable IR
// function (dense `FuncId -> wasm-index` numbering after the imports), the
// synthetic `__*` runtime helpers and builtin wrappers it needs, a passive data
// segment holding every string constant, and the entry export.

use std::collections::{HashMap, HashSet};

use ir::{GlobalInit, IrProgram, PreEval};
use wasm_encoder::{
	CodeSection, ConstExpr, DataCountSection, DataSection, ElementSection, Elements, ExportKind,
	ExportSection, Function, FunctionSection, GlobalSection, GlobalType, HeapType, ImportSection,
	Module as WasmModule, RefType, TableSection, TableType, TypeSection, ValType,
};

use crate::emit::FnEmitter;
use crate::helpers::*;
use crate::runtime::{
	host_sig, is_f64_unary_host, is_inline_builtin, scan_needs, GlobalKind, GlobalSlot, Needs,
	OptionLits, OrderingLits, Runtime, ToStringLits, WireTags,
};
use crate::scan::{collect_host_calls, collect_zero_arg_closures, scan_strings, StrPool};
use crate::types::{self, FuncTypes};
use crate::util::{variant_display, variant_tag_in};
use crate::{builtin_globals, Diagnostics, Reach};

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
