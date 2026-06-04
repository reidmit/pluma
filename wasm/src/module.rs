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
	MemorySection, MemoryType, Module as WasmModule, RefType, TableSection, TableType, TypeSection,
	ValType,
};

use crate::emit::FnEmitter;
use crate::helpers::{REGISTRY, build_builtin_wrapper, builtin_arity, close_deps, helper_for_tag};
use crate::runtime::{
	ClockKind, DomKind, GlobalKind, GlobalSlot, Helper, HelperCtx, HelperSet, IoKind, IoResultLits,
	NetImports, OptionLits, OrderingLits, RngKind, Runtime, TaskGlobals, TaskLits, ToStringLits,
	WireGlobals, WireResultLits, WireTags, clock_kind, dom_kind, host_sig, io_kind, io_uses_io4,
	is_byte_writer, is_clock_host, is_dom_host, is_f64_unary_host, is_inline_builtin, is_io_host,
	is_io_result, is_net_builtin, is_raw_writer, is_rng_host, rng_kind, scan_helpers,
	task_builtin_kind,
};
use crate::scan::{StrPool, collect_host_calls, collect_zero_arg_closures, scan_strings};
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
		// Whether the program reaches a `core.net` builtin. The seven socket ops plus
		// the reactor's `net-poll`/`net-unwatch` are registered together once, after
		// the scan (see below) — the suspending `accept`/`read`/`write` are `$task`
		// constructors driven by the scheduler, and `poll`/`unwatch` are reached only
		// from the emitted driver, so none surface as ordinary host calls here.
		let mut uses_net = false;
		for &fid in &reach.order {
			collect_host_calls(&p.functions[fid as usize].body, &builtin_g, |tag| {
				if is_net_builtin(tag) {
					uses_net = true;
					return;
				}
				if let Some(h) = helper_for_tag(tag) {
					requested.insert(h);
					return;
				}
				// `debug` is emitted inline (see `emit_debug`): it renders the value
				// via `__tostring`, concatenates the `[module:line]` prefix with
				// `__bytesconcat`, and prints the line through the `print` host import.
				if tag == "debug" {
					requested.insert(Helper::ToString);
					requested.insert(Helper::BytesConcat);
					requested.insert(Helper::MarshalSend);
					if !host_index.contains_key("print") {
						host_index.insert("print".to_string(), host_order.len() as u32);
						host_order.push("print".to_string());
					}
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
				// Byte-payload writers render their arg into scratch before the host
				// call: they need `__send_bytes` (and `__tostring` for the formatted
				// ones). They still register as ordinary host imports below.
				if is_byte_writer(tag) {
					requested.insert(Helper::MarshalSend);
					if !is_raw_writer(tag) {
						requested.insert(Helper::ToString);
					}
				}
				// Marshalled `core.io` ops encode their path/data args into scratch
				// (`__alloc`/`__store_bytes`); reads also `__load_bytes` the payload and
				// need the `io-copyout` overflow import, and `read-dir` splits names.
				if is_io_host(tag) {
					requested.insert(Helper::MarshalAlloc);
					requested.insert(Helper::MarshalStore);
					if let Some(kind) = io_kind(tag) {
						let is_read = matches!(
							kind,
							IoKind::ReadStr
								| IoKind::ReadBytes
								| IoKind::ReadFileStr
								| IoKind::ReadFileBytes
								| IoKind::ReadDir
								| IoKind::Args
								| IoKind::EnvVar
						);
						if is_read {
							requested.insert(Helper::MarshalLoad);
							if !host_index.contains_key("io-copyout") {
								host_index.insert("io-copyout".to_string(), host_order.len() as u32);
								host_order.push("io-copyout".to_string());
							}
						}
						// Both split a NUL-blob into a `$list` of `$str`.
						if matches!(kind, IoKind::ReadDir | IoKind::Args) {
							requested.insert(Helper::MarshalReadNames);
						}
					}
				}
				// `core.io` result builtins need the `__io_result` shaper + the
				// `io-last-error` channel it queries, on top of their own host import
				// (registered by the generic path just below — fall through). `uuid-parse`
				// rides this path too (it's classified as an io read).
				if is_io_result(tag) {
					requested.insert(Helper::IoResult);
					if !host_index.contains_key("io-last-error") {
						host_index.insert("io-last-error".to_string(), host_order.len() as u32);
						host_order.push("io-last-error".to_string());
					}
				}
				// `core.random`/`core.uuid` payload builders (`emit_rng`): the byte/string
				// ones write to scratch and read it back (`random-bytes` may overflow);
				// the scalars need no helpers. Their host import is registered below.
				if is_rng_host(tag) {
					match rng_kind(tag) {
						Some(RngKind::BytesN) => {
							requested.insert(Helper::MarshalAlloc);
							requested.insert(Helper::MarshalLoad);
							if !host_index.contains_key("io-copyout") {
								host_index.insert("io-copyout".to_string(), host_order.len() as u32);
								host_order.push("io-copyout".to_string());
							}
						}
						Some(RngKind::UuidStr) => {
							requested.insert(Helper::MarshalAlloc);
							requested.insert(Helper::MarshalLoad);
						}
						_ => {}
					}
				}
				// `core.time` clock imports (`emit_clock`). now/monotonic/sleep need no
				// helpers; `time-parse` marshals two strings + a scratch i64 slot and
				// shapes its `result instant string` through `__io_result` (so it needs
				// the marshalling helpers + the `io-last-error` error channel).
				if clock_kind(tag) == Some(ClockKind::Parse) {
					requested.insert(Helper::MarshalAlloc);
					requested.insert(Helper::MarshalStore);
					requested.insert(Helper::MarshalLoad);
					requested.insert(Helper::IoResult);
					if !host_index.contains_key("io-last-error") {
						host_index.insert("io-last-error".to_string(), host_order.len() as u32);
						host_order.push("io-last-error".to_string());
					}
				}
				// `core.dom` (`emit_dom`): string-carrying node ops marshal their args into
				// scratch; `dom-get-value` reads a payload back; `on-click` stows its handler
				// in the dispatch registry (the `__dom_register`/`__dom_dispatch` helpers,
				// whose dep `__list_push` and the `dom_handlers` global come in below). The
				// dom host import itself is registered by the generic path just after.
				if is_dom_host(tag) {
					match dom_kind(tag) {
						Some(DomKind::Make | DomKind::SetText | DomKind::SetAttr) => {
							requested.insert(Helper::MarshalAlloc);
							requested.insert(Helper::MarshalStore);
						}
						Some(DomKind::GetValue) => {
							requested.insert(Helper::MarshalAlloc);
							requested.insert(Helper::MarshalLoad);
						}
						Some(DomKind::Listen) => {
							requested.insert(Helper::DomRegister);
							requested.insert(Helper::DomDispatch);
						}
						_ => {}
					}
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
		// Every program exports `__entry_error`, which renders a `result.err` message
		// via `__tostring` + `__send_bytes` — request them here, *before* the
		// `float_to_str` gate below, so `__tostring`'s float-format import is registered
		// (a non-printing program would otherwise pull `__tostring` in only via
		// `close_deps`, after this gate, and miss `float_to_str`).
		requested.insert(Helper::EntryError);
		requested.insert(Helper::ToString);
		requested.insert(Helper::MarshalSend);
		// `__tostring` delegates float formatting to a host import.
		if requested.contains(&Helper::ToString) {
			host_index.insert("float_to_str".to_string(), host_order.len() as u32);
			host_order.push("float_to_str".to_string());
		}
		// `core.net`: register the whole import set together when any net builtin is
		// reachable (the sync ops shaped at the call site, the suspending ops + the
		// reactor controls driven by the scheduler). The host defines all nine
		// unconditionally, so importing the full set even when only some are used is
		// harmless; it keeps the indices a single contiguous block.
		let net_imports = uses_net.then(|| {
			let mut reg = |name: &str| -> u32 {
				let idx = host_order.len() as u32;
				host_index.insert(name.to_string(), idx);
				host_order.push(name.to_string());
				idx
			};
			NetImports {
				listen: reg("net-listen"),
				close: reg("net-close"),
				local_addr: reg("net-local-addr"),
				connect: reg("net-connect"),
				accept: reg("net-accept"),
				read: reg("net-read"),
				write: reg("net-write"),
				poll: reg("net-poll"),
				unwatch: reg("net-unwatch"),
			}
		});
		// `core.net` shapes its results through `__io_result` (the same `ok`/`err` +
		// `io-last-error` channel as `core.io`) and marshals byte payloads (addr/data,
		// the read result) through scratch — pull those helpers + the error import in.
		if uses_net {
			requested.insert(Helper::IoResult);
			requested.insert(Helper::MarshalAlloc);
			requested.insert(Helper::MarshalStore);
			requested.insert(Helper::MarshalLoad);
			if !host_index.contains_key("io-last-error") {
				host_index.insert("io-last-error".to_string(), host_order.len() as u32);
				host_order.push("io-last-error".to_string());
			}
		}
		let num_imports = host_order.len() as u32;

		// Dense FuncId -> wasm function index (imports occupy the low indices).
		let mut wasm_index: HashMap<u32, u32> = HashMap::new();
		for (i, &fid) in reach.order.iter().enumerate() {
			wasm_index.insert(fid, num_imports + i as u32);
		}

		// `fun { body }` lowers to a function with *zero* IR params, but its type is
		// `nothing -> a` (arity 1) — its call sites pass the `()` arg. A uniformly-boxed
		// dispatch would tolerate the arity mismatch, but `call_indirect` does not, so give every
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
		runtime.io_last_error = host_index.get("io-last-error").copied();
		runtime.net = net_imports;
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
						PreEval::Builtin(t, _) if builtin_arity(t).is_some() => tags.push(t.clone()),
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
		// The marshalling scratch bump cursor (`Runtime.bump`): a mutable `i32` holding
		// the next free offset in the exported linear memory. Allocated first and
		// unconditionally — the memory + this global are emitted for every module — so
		// any host-import emit site can encode `(ptr,len)` payloads without gating.
		runtime.bump = gidx;
		globals_sec.global(
			GlobalType {
				val_type: ValType::I32,
				mutable: true,
				shared: false,
			},
			&ConstExpr::i32_const(0),
		);
		gidx += 1;
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
		// The `core.dom` event-handler registry (`dom_handlers`): a mutable `(ref null
		// $list)` of handler closures, indexed by the token `dom.on-click` hands the host.
		// Allocated (and `__dom_dispatch` exported, below) only when a listener is
		// reachable — `__dom_register` lazily fills it on the first registration.
		if requested.contains(&Helper::DomDispatch) {
			let idx = gidx;
			globals_sec.global(
				GlobalType {
					val_type: ValType::Ref(RefType {
						nullable: true,
						heap_type: HeapType::Concrete(types::T_LIST),
					}),
					mutable: true,
					shared: false,
				},
				&ConstExpr::ref_null(HeapType::Concrete(types::T_LIST)),
			);
			gidx += 1;
			runtime.dom_handlers = Some(idx);
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
		// names and resolve their within-enum tags (the `option` enum). `io.env`
		// (`emit_env`) builds the same `some`/`none` variants inline, so it needs the
		// option lits populated too.
		if requested.contains(&Helper::DictLookup) || host_index.contains_key("io-env") {
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

		// `core.io` result builtins wrap their host return in `ok`/`err` via
		// `__io_result`; resolve the `result` enum's variant tags + display names.
		if requested.contains(&Helper::IoResult) {
			let res = "__prelude__.result";
			let tag_in = |name: &str| {
				p.enums
					.get(res)
					.and_then(|vs| vs.iter().position(|(n, _)| n == name))
					.map(|i| i as u32)
			};
			match (tag_in("ok"), tag_in("err")) {
				(Some(ok_tag), Some(err_tag)) => {
					runtime.ioreslits = IoResultLits {
						ok_tag,
						err_tag,
						ok_name: strpool.intern(&variant_display(res, ok_tag, &p.enums)),
						err_name: strpool.intern(&variant_display(res, err_tag, &p.enums)),
					};
				}
				_ => diags.push("`core.io` needs the `result` enum".to_string()),
			}
		}

		// Function-type interning + section building.
		let mut ftypes = FuncTypes::new();

		let mut imports = ImportSection::new();
		for tag in &host_order {
			let ty = if tag == "float_to_str" {
				ftypes.for_float_to_str()
			} else if is_byte_writer(tag) {
				ftypes.for_host_write()
			} else if tag == "io-copyout" || tag == "io-exit" {
				// Both are `(i32) -> ()`: io-copyout's `dst`, io-exit's `code`.
				ftypes.for_io_copyout()
			} else if tag == "io-last-error" {
				ftypes.for_io2()
			} else if is_io_host(tag) {
				if io_uses_io4(tag) {
					ftypes.for_io4()
				} else {
					ftypes.for_io2()
				}
			} else if is_f64_unary_host(tag) {
				ftypes.for_f64_unary()
			} else if is_clock_host(tag) {
				match clock_kind(tag) {
					// now/monotonic: `() -> i64`, same shape as `random-int`.
					Some(ClockKind::NowInstant | ClockKind::MonotonicDuration) => ftypes.for_rng_i64(),
					Some(ClockKind::Sleep) => ftypes.for_time_sleep(),
					Some(ClockKind::Parse) | None => ftypes.for_time_parse(),
				}
			} else if is_rng_host(tag) {
				match rng_kind(tag) {
					Some(RngKind::ScalarI64) => ftypes.for_rng_i64(),
					Some(RngKind::ScalarF64) => ftypes.for_rng_f64(),
					Some(RngKind::RangeI64) => ftypes.for_rng_range(),
					Some(RngKind::BytesN) => ftypes.for_rng_bytes(),
					// uuid-v4/v7: `(dst, cap) -> len`, same shape as a two-arg io read.
					Some(RngKind::UuidStr) | None => ftypes.for_io2(),
				}
			} else if is_dom_host(tag) {
				match dom_kind(tag) {
					Some(DomKind::Body) => ftypes.for_dom_body(),
					Some(DomKind::Make) => ftypes.for_dom_make(),
					Some(DomKind::Append) => ftypes.for_dom_append(),
					Some(DomKind::SetAttr) => ftypes.for_dom_set_attr(),
					Some(DomKind::SetText) => ftypes.for_dom_node_str(),
					Some(DomKind::GetValue) => ftypes.for_dom_get_value(),
					Some(DomKind::Listen) | None => ftypes.for_dom_listen(),
				}
			} else if tag == "net-listen" || tag == "net-connect" || tag == "net-accept" {
				ftypes.for_net_listen()
			} else if tag == "net-close" {
				ftypes.for_net_close()
			} else if tag == "net-local-addr" {
				ftypes.for_net_local_addr()
			} else if tag == "net-read" || tag == "net-write" {
				ftypes.for_net_rw()
			} else if tag == "net-poll" {
				ftypes.for_net_poll()
			} else if tag == "net-unwatch" {
				ftypes.for_net_unwatch()
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
		// Then the builtin method-dict wrappers (keyed by tag, not in the helper
		// catalog): each pure-compute trait method (`int-add`, `string-compare`, …)
		// gets an unbox/compute/rebox wrapper. (Builtins used as a first-class
		// *value* are wrapped earlier, in `ir::lower`, as ordinary forwarding
		// closures — they never reach this table.)
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

		// The marshalling scratch: one exported linear memory (initially one 64KiB
		// page, growable). Host imports read/write byte payloads through it instead of
		// reflecting GC `$value` fields. Emitted unconditionally — every artifact gets
		// it, so a host can always find the `"memory"` export.
		let mut memory = MemorySection::new();
		memory.memory(MemoryType {
			minimum: 1,
			maximum: None,
			memory64: false,
			shared: false,
			page_size_log2: None,
		});

		let mut exports = ExportSection::new();
		exports.export("memory", ExportKind::Memory, 0);
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
		// `__entry_error(ret) -> i32`: the host calls this on `_entry`'s return to detect
		// a `result.err` failure + read its message out of scratch (no GC reflection).
		if let Some(w) = runtime.idx(Helper::EntryError) {
			exports.export("__entry_error", ExportKind::Func, w);
		}
		// `__dom_dispatch(token) -> ()`: the browser loader calls this when a registered
		// DOM event fires, to run the handler closure stowed at `token`.
		if let Some(w) = runtime.idx(Helper::DomDispatch) {
			exports.export("__dom_dispatch", ExportKind::Func, w);
		}

		let mut data = DataSection::new();
		data.passive(strpool.bytes.iter().copied());
		let data_count = DataCountSection { count: 1 };

		let mut module = WasmModule::new();
		module.section(&types);
		module.section(&imports);
		module.section(&functions);
		module.section(&tables);
		module.section(&memory);
		module.section(&globals_sec);
		module.section(&exports);
		module.section(&elements);
		module.section(&data_count);
		module.section(&code);
		module.section(&data);
		module.finish()
	}
}
