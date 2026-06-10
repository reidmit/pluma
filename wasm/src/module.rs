// Module assembly. `Module::build` lays out the wasm module: host imports for the
// builtins a program actually calls, one defined function per reachable IR
// function (dense `FuncId -> wasm-index` numbering after the imports), the
// synthetic `__*` runtime helpers and builtin wrappers it needs, a passive data
// segment holding every string constant, and the entry export.
//
// The build is a linear pipeline; the bulkier phases live in submodules:
//   - `imports` — the host-import table, per-tag classification, import types.
//   - `globals` — the global-section assembly (scratch/wire/task/dom globals).
//   - `lits`    — the per-enum literal tables the codecs/formatters dispatch on.

mod globals;
mod imports;
mod lits;

use crate::emit::FnEmitter;
use crate::helpers::{REGISTRY, build_builtin_wrapper, builtin_arity, close_deps};
use crate::runtime::{
	Helper, HelperCtx, HelperSet, IoImports, NetImports, OffloadImports, Runtime, is_net_builtin,
	is_offload_builtin, scan_helpers,
};
use crate::scan::{StrPool, collect_host_calls, collect_zero_arg_closures, scan_strings};
use crate::types::FuncTypes;
use crate::{Diagnostics, Reach, builtin_globals};
use globals::build_globals;
use imports::{HostImports, classify_host_call, import_type};
use ir::{GlobalInit, IrProgram, PreEval};
use lits::resolve_literals;
use std::collections::{HashMap, HashSet};
use wasm_encoder::{
	CodeSection, ConstExpr, DataCountSection, DataSection, ElementSection, Elements, ExportKind,
	ExportSection, Function, FunctionSection, ImportSection, MemorySection, MemoryType,
	Module as WasmModule, RefType, TableSection, TableType, TypeSection,
};

pub(crate) struct Module;

impl Module {
	pub fn build(
		p: &IrProgram,
		reach: &Reach,
		param_shapes: &HashMap<u32, Vec<Option<ir::RecordShape>>>,
		browser: bool,
		diags: &mut Diagnostics,
	) -> Vec<u8> {
		let builtin_g = builtin_globals(p);

		// Host imports: the builtin tags actually called in reachable functions. The
		// per-tag classification (which synthetic `__*` helpers + imports each call
		// pulls in) lives in `imports::classify_host_call`.
		let mut imports = HostImports::new();

		// Synthetic `__*` helpers the program needs. Some are triggered by a named
		// builtin call (in `classify_host_call`); the rest by IR construct (added by
		// `scan_helpers` below).
		let mut requested: HelperSet = HelperSet::new();

		// Whether the program reaches a `std.sys.net` builtin. The seven socket ops plus
		// the reactor's `net-poll`/`net-unwatch` are registered together once, after the
		// scan (see below) — the suspending `accept`/`read`/`write` are `$task`
		// constructors driven by the scheduler, and `poll`/`unwatch` are reached only
		// from the emitted driver, so none surface as ordinary host calls here.
		let mut uses_net = false;
		// Whether the program reaches a `BlockingPool` offload builtin (host/src/offload.rs) — a
		// suspending op whose blocking call the scheduler offloads to a host worker thread
		// (`offload-sleep` in v0; async fs next). Gates the offload imports + the shared
		// reactor controls, like `uses_net` does for sockets.
		let mut uses_offload = false;
		// Whether the program reaches the task-local builtins. `local-get` walks the
		// scheduler's per-fiber env; `local-enter`/`-exit` only appear inside
		// `local.with`. Tracked here (like net) so their helper requests can be gated
		// on actual use.
		let mut uses_local_get = false;
		let mut uses_local_kernel = false;
		// Whether the program reaches a browser RPC stream builtin (`std.web.stream`).
		// The three `rpc-stream-*` builtins + their host channel are registered together
		// once, after the scan (like net): `rpc-stream-next` is a `$task` kind driven by
		// the scheduler, `open`/`close` are shaped at their emit sites, and the exports
		// (`__rpc_stream_alloc`/`_event`) are host-called, so none is an ordinary call.
		let mut uses_rpc_stream = false;
		// Whether the program reaches the unary browser fetch (`std.web.fetch`). It rides
		// the same host-fed channel as the stream path (`post` pulls its one reply with
		// `rpc-stream-next`), so it only adds the `WebFetchOpen` helper + its import on top
		// of the shared rpc-stream machinery.
		let mut uses_web_fetch = false;

		// Go through each reachable function...
		for &func_id in &reach.order {
			// Look it up the IR program,
			collect_host_calls(&p.functions[func_id as usize].body, &builtin_g, |tag| {
				if is_net_builtin(tag) {
					uses_net = true;
					return;
				}
				if is_offload_builtin(tag) {
					uses_offload = true;
					return;
				}
				if matches!(
					tag,
					"rpc-stream-open" | "rpc-stream-next" | "rpc-stream-close"
				) {
					uses_rpc_stream = true;
					return;
				}
				// In a browser build `web-fetch` rides the async pull channel (the
				// `WebFetch` helper, intercepted here); under the V8 sys host it falls
				// through to the blocking classifier below (`emit_web_fetch`).
				if browser && tag == "web-fetch" {
					uses_web_fetch = true;
					return;
				}
				if tag == "local-get" {
					uses_local_get = true;
					return;
				}
				if matches!(tag, "local-enter" | "local-exit") {
					uses_local_kernel = true;
					return;
				}
				classify_host_call(tag, &mut requested, &mut imports, diags);
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
			imports.register("float_to_str");
		}
		// `std.sys.net`: register the seven socket ops together when any net builtin is
		// reachable (the sync ops shaped at the call site, the suspending ops driven by the
		// scheduler). The host defines them all unconditionally, so importing the full set
		// even when only some are used is harmless; it keeps the indices a contiguous block.
		let net_imports = uses_net.then(|| NetImports {
			listen: imports.register("net-listen"),
			close: imports.register("net-close"),
			local_addr: imports.register("net-local-addr"),
			connect: imports.register("net-connect"),
			accept: imports.register("net-accept"),
			read: imports.register("net-read"),
			write: imports.register("net-write"),
		});
		// The shared offload reactor controls (`io-poll`/`io-unwatch`, host/src/offload.rs): the
		// scheduler's block step + reap drive these for *any* async-I/O client — sockets
		// (readiness) and offloaded blocking work (completion) feed one poll step. Present
		// when either kind of builtin is reached.
		let io_imports = (uses_net || uses_offload).then(|| IoImports {
			poll: imports.register("io-poll"),
			unwatch: imports.register("io-unwatch"),
		});
		// `BlockingPool` offload clients: the suspending ops whose blocking call runs on a
		// host worker thread (`offload-sleep` proving op + the async-fs ops). The host
		// defines them all unconditionally, so registering the full set is harmless.
		let offload_imports = uses_offload.then(|| OffloadImports {
			sleep: imports.register("offload-sleep"),
			op: imports.register("fs-op"),
			db: imports.register("db-op"),
		});
		// Both net and offload shape their results through `__io_result` (the same `ok`/`err`
		// + `io-last-error` channel as `std.sys.io`) and marshal byte payloads through scratch
		// — pull those helpers + the error import in when either is reachable.
		if uses_net || uses_offload {
			requested.insert(Helper::IoResult);
			requested.insert(Helper::MarshalAlloc);
			requested.insert(Helper::MarshalStore);
			requested.insert(Helper::MarshalLoad);
			imports.register("io-last-error");
		}
		// An offload-fs read produces an unknown-size payload, so it needs the read-overflow
		// drain (`io-copyout`) the sync reads use — register it when offload is reachable.
		if uses_offload {
			imports.register("io-copyout");
		}
		// Browser MVU command runtime: the long-lived `__browser_entry` (replacing the
		// run-to-completion `__task_entry`), the `__browser_resume` timer callback, and
		// the `dom-set-timeout` host import the pump calls. `BrowserEntry` pulls
		// `BrowserRun` (+ Pump/Park/…) via deps. `SpawnCommand` rides in via its builtin tag.
		if browser {
			requested.insert(Helper::BrowserEntry);
			requested.insert(Helper::BrowserResume);
			imports.register("dom-set-timeout");
		}
		// `std.web.stream`: a browser RPC subscription. The exported channel pump
		// (`__rpc_stream_alloc`/`_event`), the scratch-marshalling helpers the
		// open/event paths use, the list ops the channel registry/queue use, and the
		// two host imports (`fetch` start + abort). `RpcStreamEvent` pulls the wake
		// machinery (`__browser_run`, `__list_append`) via deps.
		if uses_rpc_stream {
			requested.insert(Helper::RpcStreamAlloc);
			requested.insert(Helper::RpcStreamEvent);
			requested.insert(Helper::RpcStreamOpen);
			requested.insert(Helper::RpcStreamClose);
			requested.insert(Helper::MarshalAlloc);
			requested.insert(Helper::MarshalStore);
			requested.insert(Helper::MarshalLoad);
			requested.insert(Helper::MarshalSend);
			requested.insert(Helper::ListPush);
			requested.insert(Helper::ListAppend);
			imports.register("rpc-stream-open");
			imports.register("rpc-stream-close");
		}
		// `std.web.fetch` unary transport in a browser build: the single-shot case of the
		// channel above. The `WebFetch` helper mints a channel + starts the async `fetch`
		// (the `web-fetch-open` import) and returns a `WEB_FETCH` task; the pump pulls the
		// one reply. Request the shared channel machinery so it stands alone even if the
		// streaming open is DCE'd out.
		if uses_web_fetch {
			requested.insert(Helper::WebFetch);
			requested.insert(Helper::RpcStreamAlloc);
			requested.insert(Helper::RpcStreamEvent);
			requested.insert(Helper::MarshalAlloc);
			requested.insert(Helper::MarshalStore);
			requested.insert(Helper::MarshalLoad);
			requested.insert(Helper::MarshalSend);
			requested.insert(Helper::ListPush);
			requested.insert(Helper::ListAppend);
			imports.register("web-fetch-open");
		}
		let num_imports = imports.len();

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
		// Every program exports `__task_entry` as `_entry`; that pulls in the whole
		// driver (`TaskDrive` + its deps) via `close_deps`. `SchedSpawn` is reached
		// only from `emit`'s `s.spawn` (not a helper dep), so request it explicitly.
		requested.insert(Helper::TaskEntry);
		requested.insert(Helper::SchedSpawn);
		requested.insert(Helper::SchedCancel);
		requested.insert(Helper::SchedCancelAfter);
		// Task-local helpers reference the scheduler globals; request them when their
		// builtins are reached.
		if uses_local_get {
			requested.insert(Helper::LocalGet);
		}
		if uses_local_kernel {
			requested.insert(Helper::LocalEnter);
			requested.insert(Helper::LocalExit);
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
		runtime.float_to_str = imports.get("float_to_str");
		runtime.io_last_error = imports.get("io-last-error");
		runtime.io_copyout = imports.get("io-copyout");
		runtime.net = net_imports;
		runtime.io = io_imports;
		runtime.offload = offload_imports;
		runtime.dom_set_timeout = imports.get("dom-set-timeout");
		runtime.rpc_stream_open = imports.get("rpc-stream-open");
		runtime.rpc_stream_close = imports.get("rpc-stream-close");
		runtime.web_fetch_open = imports.get("web-fetch-open");
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

		// Lazily-initialized globals + the wire/async/dom module-level scratch (see
		// `globals::build_globals`); sets the corresponding `Runtime` indices in place.
		let (globals_sec, gmap) = build_globals(
			p,
			&sorted_globals,
			&wasm_index,
			&methoddicts,
			&wrapper_idx,
			&requested,
			needs_wire_codec,
			&mut runtime,
		);

		// String-constant pool: one passive data segment, every `Const::Str`
		// concatenated, recorded by (offset, len).
		let mut strpool = StrPool::default();
		for &fid in &reach.order {
			scan_strings(&p.functions[fid as usize].body, &mut strpool, &p.enums);
		}

		// The per-enum literal tables the codecs/formatters dispatch on (`__tostring`'s
		// fixed strings, the `option`/`ordering`/`wire`/`result` variant tags + display
		// names); interns into `strpool`, fills `runtime`. See `lits::resolve_literals`.
		resolve_literals(
			p,
			&requested,
			&wrapper_order,
			&imports,
			needs_wire_codec,
			&mut strpool,
			&mut runtime,
			diags,
		);

		// Function-type interning + section building.
		let mut ftypes = FuncTypes::new();

		let mut import_sec = ImportSection::new();
		for tag in imports.order() {
			let ty = import_type(tag, &mut ftypes);
			import_sec.import("pluma", tag, wasm_encoder::EntityType::Function(ty));
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
				imports.index_map(),
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
		// A Browser MVU program enters through `__browser_entry` (init + pump + return,
		// long-lived); every other program through `__task_entry`, which drives the
		// task `main` returns to completion — or, for a fully sync `main`, hands its
		// plain return value straight back.
		let entry_export = if browser {
			runtime.idx(Helper::BrowserEntry)
		} else {
			runtime.idx(Helper::TaskEntry)
		};
		if let Some(w) = entry_export {
			exports.export("_entry", ExportKind::Func, w);
		}
		// `__entry_error(ret) -> i32`: the host calls this on `_entry`'s return to detect
		// a `result.err` failure + read its message out of scratch (no GC reflection).
		if let Some(w) = runtime.idx(Helper::EntryError) {
			exports.export("__entry_error", ExportKind::Func, w);
		}
		// `__browser_resume(token) -> ()`: the browser loader's `setTimeout` calls this
		// when an armed timer fires, to advance the clock + re-pump the command runtime.
		if let Some(w) = runtime.idx(Helper::BrowserResume) {
			exports.export("__browser_resume", ExportKind::Func, w);
		}
		// `__dom_dispatch(token) -> ()`: the browser loader calls this when a registered
		// DOM event fires, to run the handler closure stowed at `token`.
		if let Some(w) = runtime.idx(Helper::DomDispatch) {
			exports.export("__dom_dispatch", ExportKind::Func, w);
		}
		// `__rpc_stream_alloc(token, n) -> ptr` / `__rpc_stream_event(token, kind, ptr,
		// len)`: the browser loader reserves scratch + pushes parsed SSE events into a
		// subscription's channel (`std.web.stream`).
		if let Some(w) = runtime.idx(Helper::RpcStreamAlloc) {
			exports.export("__rpc_stream_alloc", ExportKind::Func, w);
		}
		if let Some(w) = runtime.idx(Helper::RpcStreamEvent) {
			exports.export("__rpc_stream_event", ExportKind::Func, w);
		}

		let mut data = DataSection::new();
		data.passive(strpool.bytes.iter().copied());
		let data_count = DataCountSection { count: 1 };

		let mut module = WasmModule::new();
		module.section(&types);
		module.section(&import_sec);
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
