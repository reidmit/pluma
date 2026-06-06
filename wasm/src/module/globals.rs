// Global-section assembly for `Module::build`: the scratch bump cursor, the
// lazily-initialized def/method-dict slots, and the `wire` / async-driver / DOM
// module-level scratch globals. Allocates the wasm globals, records the slot map
// the emitter reads (`gmap`), and back-fills the global indices into `Runtime`.

use crate::runtime::{
	GlobalKind, GlobalSlot, Helper, HelperSet, Runtime, TaskGlobals, WireGlobals,
};
use crate::types;
use ir::{GlobalInit, IrProgram};
use std::collections::HashMap;
use wasm_encoder::{ConstExpr, GlobalSection, GlobalType, HeapType, RefType, ValType};

/// Build the global section and the realized-slot map. Allocates (in order) the
/// scratch bump cursor, each reachable lazy def/method-dict slot, and — when
/// reachable — the `wire` codec scratch, the DOM handler registry, and the async
/// scheduler's state. Sets the corresponding `Runtime` global indices in place.
pub(super) fn build_globals(
	p: &IrProgram,
	sorted_globals: &[u32],
	wasm_index: &HashMap<u32, u32>,
	methoddicts: &[(u32, Vec<String>)],
	wrapper_idx: &HashMap<String, u32>,
	requested: &HelperSet,
	needs_wire_codec: bool,
	runtime: &mut Runtime,
) -> (GlobalSection, HashMap<u32, GlobalSlot>) {
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
	for &gid in sorted_globals {
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
	for (gid, tags) in methoddicts {
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
	// ref-typed ones (`buf`/`input`/`ctx`) start null and are pre-initialized before
	// any array op so they never trap.
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
	// The `std.web.dom` event-handler registry (`dom_handlers`): a mutable `(ref null
	// $list)` of handler closures, indexed by the token `dom.on-click` hands the host.
	// Allocated (and `__dom_dispatch` exported, later) only when a listener is
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
	// The host-fed RPC stream channel registry (`rpc_channels`): a mutable `(ref null
	// $list)` of channel records (see `rpc_chan`), indexed by the token
	// `rpc-stream-open` hands the host. Allocated only when a browser RPC subscription
	// is reachable; `emit_rpc_stream_open` lazily fills it on the first `open`.
	if requested.contains(&Helper::RpcStreamEvent) {
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
		runtime.rpc_channels = Some(idx);
	}
	// The async scheduler's module-level state: the current fiber's activation stack
	// plus the fiber/scope tables, ready deque, timers, and the pump's output
	// channel. Allocated for every program (the scheduler always drives `main`).
	// Ref-typed globals start null (set on each `run`).
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
		current_fiber: task_global(ValType::I32, &zero_i32),
	};

	(globals_sec, gmap)
}
