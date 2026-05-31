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
use crate::helpers::{
	REGISTRY, build_builtin_wrapper, build_host_value_wrapper, builtin_arity, close_deps,
	helper_for_tag,
};
use crate::runtime::{
	GlobalKind, GlobalSlot, Helper, HelperCtx, HelperSet, OptionLits, OrderingLits, Runtime,
	TaskGlobals, TaskLits, ToStringLits, WireGlobals, WireResultLits, WireTags, host_sig,
	is_f64_unary_host, is_inline_builtin, scan_helpers, task_builtin_kind,
};
use crate::scan::{
	StrPool, builtin_var_tags, collect_host_calls, collect_zero_arg_closures, scan_strings,
	value_used_builtin_vars,
};
use crate::types::{self, FuncTypes};
use crate::util::{variant_display, variant_tag_in};
use crate::{Diagnostics, Reach, builtin_globals};

pub(crate) struct Module;

impl Module {
	pub fn build(
		p: &IrProgram,
		reach: &Reach,
		param_shapes: &HashMap<u32, Vec<Option<ir::RecordShape>>>,
		is_async: bool,
		diags: &mut Diagnostics,
	) -> Vec<u8> {
		let builtin_g = builtin_globals(p);

		// Host imports: the builtin tags actually called in reachable functions.
		// `to-string` is special — it's implemented in wasm (`__tostring`), not
		// imported — so route it to a flag rather than the import table.
		let mut host_index: HashMap<String, u32> = HashMap::new();
		let mut host_order: Vec<String> = Vec::new();
		// Synthetic `__*` helpers the program needs. Some are triggered by a named
		// builtin call (via `helper_for_tag` here); the rest by IR construct (added
		// by `scan_helpers` below).
		let mut requested: HelperSet = HelperSet::new();
		for &fid in &reach.order {
			collect_host_calls(&p.functions[fid as usize].body, &builtin_g, |tag| {
				if let Some(h) = helper_for_tag(tag) {
					requested.insert(h);
					return;
				}
				// Pure-compute builtins emitted inline at the call site (no import).
				if is_inline_builtin(tag) {
					return;
				}
				// `task.*` / `scope-new`/`scope-next` build a `$task` inline (no import);
				// the side-effecting scope-kernel ops call driver helpers — both are
				// handled in `emit`, never as host imports.
				if task_builtin_kind(tag).is_some()
					|| matches!(tag, "scope-spawn" | "scope-cancel" | "scope-cancel-after")
				{
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
		// A builtin used as a first-class value (e.g. `list.each xs print`) is never
		// directly called, so `collect_host_calls` above misses its import — but its
		// value-wrapper still invokes it. Register those host imports now, before the
		// import count is fixed.
		for &fid in &reach.order {
			let body = &p.functions[fid as usize].body;
			let vt = builtin_var_tags(body, &builtin_g);
			for v in value_used_builtin_vars(body, &vt) {
				let tag = vt[&v].clone();
				if host_sig(&tag).is_some() && !host_index.contains_key(&tag) {
					host_index.insert(tag.clone(), host_order.len() as u32);
					host_order.push(tag);
				}
			}
		}
		// `__tostring` delegates float formatting to a host import.
		if requested.contains(&Helper::ToString) {
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
		// Add the construct-triggered helpers (`==`, field access, spread, …) to the
		// call-triggered ones above, then pull in every transitive dependency so each
		// present helper's builder finds the helpers it references.
		for &fid in &reach.order {
			scan_helpers(&p.functions[fid as usize].body, &mut requested);
		}
		// `wire-decode`'s call site wraps the decoded value via `__wire_result`
		// (not referenced by `__wire_dec` itself, so it isn't a dependency).
		if requested.contains(&Helper::WireDec) {
			requested.insert(Helper::WireResult);
		}
		// An async program exports `__task_entry` as `_entry`; that pulls in the whole
		// driver (`TaskDrive` + its deps) via `close_deps`. `SchedSpawn` is reached
		// only from `emit`'s `s.spawn` (not a helper dep), so request it explicitly.
		if is_async {
			requested.insert(Helper::TaskEntry);
			requested.insert(Helper::SchedSpawn);
			requested.insert(Helper::SchedCancel);
			requested.insert(Helper::SchedCancelAfter);
		}
		close_deps(&mut requested);
		// The `wire` encode/decode codec threads its recursive state through
		// module-level mutable globals; allocate them once when either is reachable.
		let needs_wire_codec =
			requested.contains(&Helper::WireEnc) || requested.contains(&Helper::WireDec);
		// Assign each needed helper a wasm index, walking `REGISTRY` (= `Helper`)
		// order — the same order emission replays below.
		let mut runtime = Runtime::default();
		// `__task_entry` calls the real IR entry, then drives the task it returns.
		runtime.entry_idx = wasm_index.get(&p.entry.0).copied();
		let mut next_synth = synth_base;
		for def in &REGISTRY {
			if requested.contains(&def.id) {
				runtime.helpers.set(def.id, next_synth);
				next_synth += 1;
			}
		}
		runtime.float_to_str = host_index.get("float_to_str").copied();
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

		// Builtins used as first-class values need a wrapper closure too. Collect
		// their tags after the method-dict ones (host-import builtins like `print`;
		// each becomes a `(env, arg) -> value` host-value wrapper below).
		for &fid in &reach.order {
			let body = &p.functions[fid as usize].body;
			let vt = builtin_var_tags(body, &builtin_g);
			for v in value_used_builtin_vars(body, &vt) {
				let tag = &vt[&v];
				if host_sig(tag).is_some() && !wrapper_idx.contains_key(tag) {
					wrapper_idx.insert(tag.clone(), wrapper_base + wrapper_order.len() as u32);
					wrapper_order.push(tag.clone());
				}
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
		// The `wire` codec's scratch globals (heterogeneous types, so not via
		// `alloc_slot`). Reset at each `wire-encode`/`wire-decode` call site; the
		// ref-typed ones (`buf`/`input`/`ctx`) start null and are pre-initialized
		// before any array op so they never trap.
		if needs_wire_codec {
			let mut wire_global = |val_type: ValType, init: &ConstExpr| -> u32 {
				let idx = gidx;
				globals_sec.global(
					GlobalType {
						val_type,
						mutable: true,
						shared: false,
					},
					init,
				);
				gidx += 1;
				idx
			};
			let null_bytes = ConstExpr::ref_null(HeapType::Concrete(types::T_BYTES));
			let null_arr = ConstExpr::ref_null(HeapType::Concrete(types::T_VALARRAY));
			let zero_i32 = ConstExpr::i32_const(0);
			let zero_i64 = ConstExpr::i64_const(0);
			let nullable = |t: u32| {
				ValType::Ref(RefType {
					nullable: true,
					heap_type: HeapType::Concrete(t),
				})
			};
			runtime.wireg = WireGlobals {
				buf: wire_global(nullable(types::T_BYTES), &null_bytes),
				len: wire_global(ValType::I32, &zero_i32),
				input: wire_global(nullable(types::T_BYTES), &null_bytes),
				pos: wire_global(ValType::I32, &zero_i32),
				err: wire_global(ValType::I32, &zero_i32),
				errval: wire_global(ValType::I64, &zero_i64),
				ctx: wire_global(nullable(types::T_VALARRAY), &null_arr),
				ctxlen: wire_global(ValType::I32, &zero_i32),
			};
		}
		// The async scheduler's module-level state: the current fiber's activation
		// stack plus the fiber/scope tables, ready deque, timers, and the pump's
		// output channel. Ref-typed globals start null (set on each `run`).
		if is_async {
			let mut task_global = |val_type: ValType, init: &ConstExpr| -> u32 {
				let idx = gidx;
				globals_sec.global(
					GlobalType {
						val_type,
						mutable: true,
						shared: false,
					},
					init,
				);
				gidx += 1;
				idx
			};
			let null_arr = ConstExpr::ref_null(HeapType::Concrete(types::T_VALARRAY));
			let null_val = ConstExpr::ref_null(HeapType::Concrete(types::T_VALUE));
			let zero_i32 = ConstExpr::i32_const(0);
			let zero_i64 = ConstExpr::i64_const(0);
			let arr = ValType::Ref(RefType {
				nullable: true,
				heap_type: HeapType::Concrete(types::T_VALARRAY),
			});
			let val = types::value_ref();
			runtime.taskg = TaskGlobals {
				act: task_global(arr, &null_arr),
				actlen: task_global(ValType::I32, &zero_i32),
				fibers: task_global(val, &null_val),
				scopes: task_global(val, &null_val),
				ready: task_global(val, &null_val),
				rhead: task_global(ValType::I32, &zero_i32),
				timers: task_global(val, &null_val),
				pending: task_global(val, &null_val),
				now: task_global(ValType::I64, &zero_i64),
				root_kind: task_global(ValType::I32, &zero_i32),
				root_val: task_global(val, &null_val),
				out_kind: task_global(ValType::I32, &zero_i32),
				out_okerr: task_global(ValType::I32, &zero_i32),
				out_val: task_global(val, &null_val),
				out_arg: task_global(ValType::I32, &zero_i32),
				out_arg64: task_global(ValType::I64, &zero_i64),
			};
		}

		// String-constant pool: one passive data segment, every `Const::Str`
		// concatenated, recorded by (offset, len).
		let mut strpool = StrPool::default();
		for &fid in &reach.order {
			scan_strings(&p.functions[fid as usize].body, &mut strpool, &p.enums);
		}
		// `__tostring`'s fixed literals go in the same data segment.
		if requested.contains(&Helper::ToString) {
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
		if requested.contains(&Helper::DictLookup) {
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
		if requested.contains(&Helper::WireFp) || needs_wire_codec {
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
		// `wire-decode` wraps its result in `ok`/`err`; resolve the `result` and
		// `wire-error` variant tags + display names `__wire_result` builds.
		if requested.contains(&Helper::WireDec) {
			let res = "__prelude__.result";
			let werr = "__prelude__.wire-error";
			let tag_in = |qual: &str, name: &str| {
				p.enums
					.get(qual)
					.and_then(|vs| vs.iter().position(|(n, _)| n == name))
					.map(|i| i as u32)
			};
			match (tag_in(res, "ok"), tag_in(res, "err")) {
				(Some(ok_tag), Some(err_tag)) => {
					// `wire-error` variants, indexed by error code minus one.
					let err_names = [
						"unexpected-end",
						"invalid-tag",
						"invalid-utf8",
						"trailing-bytes",
						"malformed",
					];
					let mut errors = [(0u32, (0u32, 0u32)); 5];
					let mut ok = true;
					for (i, name) in err_names.iter().enumerate() {
						match tag_in(werr, name) {
							Some(t) => errors[i] = (t, strpool.intern(&variant_display(werr, t, &p.enums))),
							None => ok = false,
						}
					}
					if ok {
						runtime.wirelits = WireResultLits {
							ok_tag,
							err_tag,
							ok_name: strpool.intern(&variant_display(res, ok_tag, &p.enums)),
							err_name: strpool.intern(&variant_display(res, err_tag, &p.enums)),
							errors,
						};
					} else {
						diags.push("`wire.decode` needs the `wire-error` enum variants".to_string());
					}
				}
				_ => diags.push("`wire.decode` needs the `result` enum".to_string()),
			}
		}

		// The async driver builds `result`/`option` variants (`task.attempt`,
		// `s.next`, root failure) and scans poll states for their `__defers` field.
		if is_async {
			let res = "__prelude__.result";
			let opt = "__prelude__.option";
			let tag_in = |qual: &str, name: &str| {
				p.enums
					.get(qual)
					.and_then(|vs| vs.iter().position(|(n, _)| n == name))
					.map(|i| i as u32)
			};
			match (
				tag_in(res, "ok"),
				tag_in(res, "err"),
				tag_in(opt, "some"),
				tag_in(opt, "none"),
			) {
				(Some(ok_tag), Some(err_tag), Some(some_tag), Some(none_tag)) => {
					runtime.tasklits = TaskLits {
						ok_tag,
						err_tag,
						ok_name: strpool.intern(&variant_display(res, ok_tag, &p.enums)),
						err_name: strpool.intern(&variant_display(res, err_tag, &p.enums)),
						some_tag,
						none_tag,
						some_name: strpool.intern(&variant_display(opt, some_tag, &p.enums)),
						none_name: strpool.intern(&variant_display(opt, none_tag, &p.enums)),
						defers_name: strpool.intern("__defers"),
						cancelled_msg: strpool.intern("scope cancelled"),
					};
				}
				_ => diags.push("async runtime needs the `result` + `option` enums".to_string()),
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
				fid,
				&wasm_index,
				&host_index,
				&builtin_g,
				&wrapper_idx,
				&gmap,
				&runtime,
				&strpool,
				&p.enums,
				&mut ftypes,
				param_shapes,
				extra_params,
				diags,
			);
			let func = em.emit();
			code.function(&func);
		}
		// Append the synthetic helpers after the IR functions, walking `REGISTRY` in
		// the same order their indices were assigned above. Each builder receives its
		// own index, the resolved deps/literals, and the type interner via `HelperCtx`.
		for def in &REGISTRY {
			if let Some(self_idx) = runtime.idx(def.id) {
				functions.function(def.fn_type.resolve(&mut ftypes));
				let mut ctx = HelperCtx {
					self_idx,
					rt: &runtime,
					ftypes: &mut ftypes,
				};
				code.function(&(def.build)(&mut ctx));
			}
		}
		// Then the builtin wrappers (keyed by tag, not in the helper catalog). A
		// host-import builtin used as a value (`print`, …) gets a `(env, arg) ->
		// value` host-value wrapper; the pure-compute ones (method-dict methods) get
		// the unbox/compute/rebox wrapper.
		for tag in &wrapper_order {
			if host_sig(tag).is_some() {
				functions.function(ftypes.for_arity(1));
				let host_idx = host_index[tag];
				code.function(&build_host_value_wrapper(host_idx));
			} else {
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
		// An async program enters through `__task_entry` (which drives `main`'s
		// returned task); a sync program enters the IR entry directly.
		let entry_export = if is_async {
			runtime.idx(Helper::TaskEntry)
		} else {
			wasm_index.get(&p.entry.0).copied()
		};
		if let Some(w) = entry_export {
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
