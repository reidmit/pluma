// Synthetic runtime-helper builders. Each `build_*_fn` emits one self-contained
// `wasm_encoder::Function` over the GC `$value` layout — the inline-WASM routines
// the assembler appends after the IR functions (only those a reachable program
// needs). They take each other's already-resolved wasm indices, never call each
// other at the Rust level, so they live one-domain-per-file.
//
// `REGISTRY` is the single catalog tying it together: one row per `Helper`, in
// `Helper` order, carrying that helper's wasm function type, its dependencies,
// and a builder thunk. `Module::build` walks it twice — to allocate indices, then
// to emit — so adding a helper is one row here plus its `build_*_fn` (no scattered
// field/branch/dependency edits).

use wasm_encoder::Function;

use crate::runtime::{Helper, Helper as H, HelperCtx, HelperSet, Ty};

mod bytes;
mod dict;
mod dom;
mod eq;
mod io;
mod list;
mod marshal;
mod record;
mod task;
mod tostring;
mod wat;
mod wire;
mod wrapper;

// The method-dict builtin wrappers aren't `Helper`s (they're keyed by builtin
// tag, not in the fixed catalog); `Module::build` drives them directly.
pub(crate) use wrapper::{build_builtin_wrapper, builtin_arity};

/// One synthetic helper: how it's typed, what it depends on, and how it's built.
pub(crate) struct HelperDef {
	pub(crate) id: Helper,
	/// The helper's own wasm function type.
	pub(crate) fn_type: Ty,
	/// Helpers whose wasm indices this one's builder needs (pulled into the
	/// program transitively by `close_deps`, so `HelperCtx::dep` never misses).
	pub(crate) deps: &'static [Helper],
	/// Emit the helper's body, given its resolved index, deps, and the interner.
	pub(crate) build: fn(&mut HelperCtx) -> Function,
}

/// The helper catalog, in `Helper` discriminant order (so `REGISTRY[h as usize]`
/// is `h`'s row — checked by the test below, relied on by `close_deps`).
pub(crate) static REGISTRY: [HelperDef; Helper::COUNT] = [
	HelperDef {
		id: H::Eq,
		fn_type: Ty::Eq,
		deps: &[H::DictEq],
		build: |c| eq::build_eq_fn(c.self_idx, c.dep(H::DictEq)),
	},
	HelperDef {
		id: H::GetField,
		fn_type: Ty::Helper(2),
		deps: &[H::Eq],
		build: |c| record::build_getfield_fn(c.dep(H::Eq)),
	},
	HelperDef {
		id: H::RecordUpdate,
		fn_type: Ty::Helper(3),
		deps: &[H::Eq],
		build: |c| record::build_record_update_fn(c.dep(H::Eq)),
	},
	HelperDef {
		id: H::ListTail,
		fn_type: Ty::Helper(2),
		deps: &[],
		build: |_| list::build_list_tail_fn(),
	},
	HelperDef {
		id: H::ArrConcat,
		fn_type: Ty::ArrConcat,
		deps: &[],
		build: |_| list::build_arrconcat_fn(),
	},
	HelperDef {
		id: H::BytesConcat,
		fn_type: Ty::BytesConcat,
		deps: &[],
		build: |_| bytes::build_bytesconcat_fn(),
	},
	HelperDef {
		id: H::ToString,
		fn_type: Ty::Helper(1),
		deps: &[
			H::IntStr,
			H::BytesConcat,
			H::DictEntries,
			H::MarshalAlloc,
			H::MarshalLoad,
		],
		build: |c| {
			tostring::build_tostring_fn(
				c.self_idx,
				c.dep(H::IntStr),
				c.dep(H::BytesConcat),
				c.float_to_str(),
				c.dep(H::DictEntries),
				c.dep(H::MarshalAlloc),
				c.dep(H::MarshalLoad),
				c.rt.bump,
				c.rt.lits,
			)
		},
	},
	HelperDef {
		id: H::IntStr,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |_| tostring::build_int_str_fn(),
	},
	HelperDef {
		id: H::ListBuild,
		fn_type: Ty::Helper(2),
		deps: &[],
		build: |c| list::build_list_build_fn(c.arity(1)),
	},
	HelperDef {
		id: H::ListCollect,
		fn_type: Ty::Helper(2),
		deps: &[],
		build: |c| list::build_list_collect_fn(c.arity(1)),
	},
	HelperDef {
		id: H::ListPush,
		fn_type: Ty::Helper(2),
		deps: &[],
		build: |_c| list::build_list_push_fn(),
	},
	HelperDef {
		id: H::BytesBuild,
		fn_type: Ty::Helper(2),
		deps: &[],
		build: |c| bytes::build_bytes_build_fn(c.arity(1)),
	},
	HelperDef {
		id: H::DictInsert,
		fn_type: Ty::Helper(3),
		deps: &[H::Hash, H::CnodeLookup, H::CnodeInsert],
		build: |c| {
			dict::build_dict_insert_fn(c.dep(H::Hash), c.dep(H::CnodeLookup), c.dep(H::CnodeInsert))
		},
	},
	HelperDef {
		id: H::DictLookup,
		fn_type: Ty::Helper(2),
		deps: &[H::DictFind],
		build: |c| dict::build_dict_lookup_fn(c.dep(H::DictFind), c.rt.opt),
	},
	HelperDef {
		id: H::DictRemove,
		fn_type: Ty::Helper(2),
		deps: &[H::Hash, H::CnodeLookup, H::CnodeRemove],
		build: |c| {
			dict::build_dict_remove_fn(c.dep(H::Hash), c.dep(H::CnodeLookup), c.dep(H::CnodeRemove))
		},
	},
	HelperDef {
		id: H::DictMap,
		fn_type: Ty::Helper(2),
		deps: &[H::Hash, H::CnodeTInsert, H::DictEntries],
		build: |c| {
			let arity1 = c.arity(1);
			dict::build_dict_map_fn(
				c.dep(H::Hash),
				c.dep(H::CnodeTInsert),
				c.dep(H::DictEntries),
				arity1,
			)
		},
	},
	HelperDef {
		id: H::DictFilter,
		fn_type: Ty::Helper(2),
		deps: &[H::Hash, H::CnodeTInsert, H::DictEntries],
		build: |c| {
			let arity2 = c.arity(2);
			dict::build_dict_filter_fn(
				c.dep(H::Hash),
				c.dep(H::CnodeTInsert),
				c.dep(H::DictEntries),
				arity2,
			)
		},
	},
	HelperDef {
		id: H::Hash,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| dict::build_hash_fn(c.self_idx),
	},
	HelperDef {
		id: H::DictEmpty,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |_c| dict::build_dict_empty_fn(),
	},
	HelperDef {
		id: H::DictFind,
		fn_type: Ty::Helper(2),
		deps: &[H::Hash, H::CnodeLookup],
		build: |c| dict::build_dict_find_fn(c.dep(H::Hash), c.dep(H::CnodeLookup)),
	},
	HelperDef {
		id: H::DictEq,
		fn_type: Ty::Eq,
		deps: &[H::Eq, H::DictFind, H::DictEntries],
		build: |c| dict::build_dict_eq_fn(c.dep(H::Eq), c.dep(H::DictFind), c.dep(H::DictEntries)),
	},
	HelperDef {
		id: H::DictSize,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |_c| dict::build_dict_size_fn(),
	},
	HelperDef {
		id: H::DictEntries,
		fn_type: Ty::Helper(1),
		deps: &[H::CnodeCollect],
		build: |c| dict::build_dict_entries_fn(c.dep(H::CnodeCollect)),
	},
	HelperDef {
		id: H::DictUpdate,
		fn_type: Ty::Helper(3),
		deps: &[H::Hash, H::CnodeLookup, H::CnodeInsert],
		build: |c| {
			let arity1 = c.arity(1);
			dict::build_dict_update_fn(
				c.dep(H::Hash),
				c.dep(H::CnodeLookup),
				c.dep(H::CnodeInsert),
				arity1,
				c.rt.opt,
			)
		},
	},
	HelperDef {
		id: H::DictClear,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |_c| dict::build_dict_clear_fn(),
	},
	HelperDef {
		id: H::CnodeLookup,
		fn_type: Ty::Helper(4),
		deps: &[H::Eq],
		build: |c| dict::build_cnode_lookup_fn(c.self_idx, c.dep(H::Eq)),
	},
	HelperDef {
		id: H::CnodeInsert,
		fn_type: Ty::Helper(5),
		deps: &[H::Eq, H::CnodeMerge],
		build: |c| dict::build_cnode_insert_fn(c.self_idx, c.dep(H::Eq), c.dep(H::CnodeMerge)),
	},
	HelperDef {
		id: H::CnodeMerge,
		fn_type: Ty::Helper(3),
		deps: &[],
		build: |c| dict::build_cnode_merge_fn(c.self_idx),
	},
	HelperDef {
		id: H::CnodeRemove,
		fn_type: Ty::Helper(4),
		deps: &[H::Eq],
		build: |c| dict::build_cnode_remove_fn(c.self_idx, c.dep(H::Eq)),
	},
	HelperDef {
		id: H::CnodeCollect,
		fn_type: Ty::Helper(2),
		deps: &[H::ListPush],
		build: |c| dict::build_cnode_collect_fn(c.self_idx, c.dep(H::ListPush)),
	},
	HelperDef {
		id: H::CnodeTInsert,
		fn_type: Ty::Helper(6),
		deps: &[H::Eq, H::CnodeMerge],
		build: |c| dict::build_cnode_tinsert_fn(c.self_idx, c.dep(H::Eq), c.dep(H::CnodeMerge)),
	},
	HelperDef {
		id: H::CnodeCount,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| dict::build_cnode_count_fn(c.self_idx),
	},
	HelperDef {
		id: H::DictFromEntries,
		fn_type: Ty::Helper(1),
		deps: &[H::Hash, H::CnodeTInsert, H::CnodeCount],
		build: |c| {
			dict::build_dict_from_entries_fn(c.dep(H::Hash), c.dep(H::CnodeTInsert), c.dep(H::CnodeCount))
		},
	},
	HelperDef {
		id: H::DictMintToken,
		fn_type: Ty::Helper(0),
		deps: &[],
		build: |_c| dict::build_dict_mint_token_fn(),
	},
	HelperDef {
		id: H::DictInsertInto,
		fn_type: Ty::Helper(4),
		deps: &[H::Hash, H::CnodeLookup, H::CnodeTInsert],
		build: |c| {
			dict::build_dict_insert_into_fn(
				c.dep(H::Hash),
				c.dep(H::CnodeLookup),
				c.dep(H::CnodeTInsert),
			)
		},
	},
	HelperDef {
		id: H::WireFp,
		fn_type: Ty::WireMixVal,
		deps: &[H::WireMixStr, H::WireMixLen],
		build: |c| {
			wire::build_wire_fp_fn(
				c.self_idx,
				c.dep(H::WireMixStr),
				c.dep(H::WireMixLen),
				c.rt.wire,
			)
		},
	},
	HelperDef {
		id: H::WireMixStr,
		fn_type: Ty::WireMixVal,
		deps: &[H::WireMixLen],
		build: |c| wire::build_wire_mix_str_fn(c.dep(H::WireMixLen)),
	},
	HelperDef {
		id: H::WireMixLen,
		fn_type: Ty::WireMixLen,
		deps: &[],
		build: |_| wire::build_wire_mix_len_fn(),
	},
	HelperDef {
		id: H::WirePush,
		fn_type: Ty::WirePush,
		deps: &[],
		build: |c| wire::build_wire_push_fn(c.rt.wireg),
	},
	HelperDef {
		id: H::WireUvarint,
		fn_type: Ty::WireUvarint,
		deps: &[H::WirePush],
		build: |c| wire::build_wire_uvarint_fn(c.dep(H::WirePush)),
	},
	HelperDef {
		id: H::WireCtxPut,
		fn_type: Ty::Helper(2),
		deps: &[],
		build: |c| wire::build_wire_ctxput_fn(c.rt.wireg),
	},
	HelperDef {
		id: H::WireCtxGet,
		fn_type: Ty::Helper(1),
		deps: &[H::Eq],
		build: |c| wire::build_wire_ctxget_fn(c.dep(H::Eq), c.rt.wireg),
	},
	HelperDef {
		id: H::WireEnc,
		fn_type: Ty::WireEnc,
		deps: &[
			H::WirePush,
			H::WireUvarint,
			H::WireCtxPut,
			H::WireCtxGet,
			H::WireEncVariant,
			H::WireEncDict,
		],
		build: |c| {
			wire::build_wire_enc_fn(
				c.self_idx,
				c.dep(H::WirePush),
				c.dep(H::WireUvarint),
				c.dep(H::WireCtxPut),
				c.dep(H::WireCtxGet),
				c.dep(H::WireEncVariant),
				c.dep(H::WireEncDict),
				c.rt.wire,
			)
		},
	},
	HelperDef {
		id: H::WireEncVariant,
		fn_type: Ty::WireEnc,
		deps: &[H::WireEnc, H::WireUvarint],
		build: |c| wire::build_wire_enc_variant_fn(c.dep(H::WireEnc), c.dep(H::WireUvarint)),
	},
	HelperDef {
		id: H::WireRByte,
		fn_type: Ty::WireRByte,
		deps: &[],
		build: |c| wire::build_wire_rbyte_fn(c.rt.wireg),
	},
	HelperDef {
		id: H::WireRUvarint,
		fn_type: Ty::WireRUvarint,
		deps: &[H::WireRByte],
		build: |c| wire::build_wire_ruvarint_fn(c.dep(H::WireRByte), c.rt.wireg),
	},
	HelperDef {
		id: H::WireDisp,
		fn_type: Ty::Helper(2),
		deps: &[H::BytesConcat],
		build: |c| wire::build_wire_disp_fn(c.dep(H::BytesConcat)),
	},
	HelperDef {
		id: H::WireDecVariant,
		fn_type: Ty::Helper(2),
		deps: &[H::WireRUvarint, H::WireDec, H::WireDisp],
		build: |c| {
			wire::build_wire_dec_variant_fn(
				c.dep(H::WireRUvarint),
				c.dep(H::WireDec),
				c.dep(H::WireDisp),
				c.rt.wireg,
			)
		},
	},
	HelperDef {
		id: H::WireDec,
		fn_type: Ty::Helper(1),
		deps: &[
			H::WireRUvarint,
			H::WireRByte,
			H::WireCtxPut,
			H::WireCtxGet,
			H::WireDecVariant,
			H::DictEmpty,
			H::DictInsert,
		],
		build: |c| {
			wire::build_wire_dec_fn(
				c.self_idx,
				c.dep(H::WireRUvarint),
				c.dep(H::WireRByte),
				c.dep(H::WireCtxPut),
				c.dep(H::WireCtxGet),
				c.dep(H::WireDecVariant),
				c.dep(H::DictEmpty),
				c.dep(H::DictInsert),
				c.rt.wireg,
				c.rt.wire,
			)
		},
	},
	HelperDef {
		id: H::WireResult,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| wire::build_wire_result_fn(c.rt.wireg, c.rt.wirelits),
	},
	HelperDef {
		id: H::WireBCmp,
		fn_type: Ty::Eq,
		deps: &[],
		build: |_| wire::build_wire_bcmp_fn(),
	},
	HelperDef {
		id: H::WireEncDict,
		fn_type: Ty::WireEnc,
		deps: &[
			H::WireEnc,
			H::WireUvarint,
			H::WirePush,
			H::WireBCmp,
			H::DictEntries,
		],
		build: |c| {
			wire::build_wire_enc_dict_fn(
				c.dep(H::WireEnc),
				c.dep(H::WireUvarint),
				c.dep(H::WirePush),
				c.dep(H::WireBCmp),
				c.dep(H::DictEntries),
				c.rt.wireg,
			)
		},
	},
	HelperDef {
		id: H::RecordRest,
		fn_type: Ty::Helper(2),
		deps: &[H::Eq],
		build: |c| record::build_record_rest_fn(c.dep(H::Eq)),
	},
	HelperDef {
		id: H::RunDefers,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| {
			// Defer thunks are `fun { … }` (a phantom-unit-param "zero-arg" closure,
			// wasm arity 1), so the run helper calls them with `(env, unit)`.
			let thunk_ty = c.arity(1);
			list::build_run_defers_fn(thunk_ty)
		},
	},
	HelperDef {
		id: H::IoResult,
		fn_type: Ty::Helper(1),
		deps: &[H::MarshalAlloc, H::MarshalLoad],
		build: |c| {
			io::build_io_result_fn(
				c.io_last_error(),
				c.dep(H::MarshalAlloc),
				c.dep(H::MarshalLoad),
				c.rt.bump,
				c.rt.ioreslits,
			)
		},
	},
	HelperDef {
		id: H::TaskDrive,
		fn_type: Ty::Helper(1),
		deps: &[
			H::Pump,
			H::FiberCompleted,
			H::CancelScope,
			H::Park,
			H::RunTimers,
			H::ListAppend,
		],
		build: |c| {
			task::build_run_task_fn(
				c.dep(H::Pump),
				c.dep(H::FiberCompleted),
				c.dep(H::CancelScope),
				c.dep(H::Park),
				c.dep(H::RunTimers),
				c.dep(H::ListAppend),
				c.rt.net,
				c.rt.taskg,
				c.rt.tasklits,
			)
		},
	},
	HelperDef {
		id: H::PollStep,
		fn_type: Ty::Helper(3),
		deps: &[H::PollDefersList],
		build: |c| {
			let arity2 = c.arity(2);
			task::build_poll_step_fn(c.dep(H::PollDefersList), arity2)
		},
	},
	HelperDef {
		id: H::PollDefersList,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| {
			let arity1 = c.arity(1);
			task::build_poll_defers_list_fn(arity1)
		},
	},
	HelperDef {
		id: H::PollDefersState,
		fn_type: Ty::Helper(1),
		deps: &[H::Eq, H::PollDefersList],
		build: |c| {
			task::build_poll_defers_state_fn(
				c.dep(H::Eq),
				c.dep(H::PollDefersList),
				c.rt.tasklits.defers_name,
			)
		},
	},
	HelperDef {
		id: H::ActPush,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| task::build_act_push_fn(c.rt.taskg),
	},
	HelperDef {
		id: H::TaskEntry,
		fn_type: Ty::Helper(1),
		deps: &[H::TaskDrive],
		build: |c| {
			let entry = c.rt.entry_idx.expect("async program needs an entry index");
			task::build_task_entry_fn(entry, c.dep(H::TaskDrive))
		},
	},
	HelperDef {
		id: H::Pump,
		fn_type: Ty::Helper(3),
		deps: &[
			H::PollStep,
			H::PollDefersState,
			H::ActPush,
			H::StartScope,
			H::DrainNext,
		],
		build: |c| {
			let arity1 = c.arity(1);
			// The suspending net ops marshal byte payloads through scratch; bundle the
			// helper/global indices (present exactly when net is reachable).
			let net_m = c.rt.net.map(|_| crate::runtime::NetMarshal {
				alloc: c.dep(H::MarshalAlloc),
				store: c.dep(H::MarshalStore),
				load: c.dep(H::MarshalLoad),
				io_result: c.dep(H::IoResult),
				bump: c.rt.bump,
			});
			task::build_pump_fn(
				c.dep(H::PollStep),
				c.dep(H::PollDefersState),
				c.dep(H::ActPush),
				c.dep(H::StartScope),
				c.dep(H::DrainNext),
				arity1,
				c.rt.net,
				net_m,
				c.rt.taskg,
				c.rt.tasklits,
			)
		},
	},
	HelperDef {
		id: H::StartScope,
		fn_type: Ty::Helper(3),
		deps: &[H::ListAppend],
		build: |c| {
			let arity1 = c.arity(1);
			task::build_start_scope_fn(c.dep(H::ListAppend), arity1, c.rt.taskg)
		},
	},
	HelperDef {
		id: H::SchedSpawn,
		fn_type: Ty::Helper(2),
		deps: &[H::ListAppend],
		build: |c| task::build_sched_spawn_fn(c.dep(H::ListAppend), c.rt.taskg),
	},
	HelperDef {
		id: H::FiberCompleted,
		fn_type: Ty::Helper(3),
		deps: &[H::OnBodyDone, H::OnChildDone],
		build: |c| {
			task::build_fiber_completed_fn(c.dep(H::OnBodyDone), c.dep(H::OnChildDone), c.rt.taskg)
		},
	},
	HelperDef {
		id: H::OnBodyDone,
		fn_type: Ty::Helper(3),
		deps: &[H::ReapFiber, H::TryFinalizeScope],
		build: |c| {
			task::build_on_body_done_fn(c.dep(H::ReapFiber), c.dep(H::TryFinalizeScope), c.rt.taskg)
		},
	},
	HelperDef {
		id: H::OnChildDone,
		fn_type: Ty::Helper(4),
		deps: &[H::CancelScope, H::TryFinalizeScope, H::ListAppend],
		build: |c| {
			task::build_on_child_done_fn(
				c.dep(H::CancelScope),
				c.dep(H::TryFinalizeScope),
				c.dep(H::ListAppend),
				c.rt.taskg,
				c.rt.tasklits,
			)
		},
	},
	HelperDef {
		id: H::CancelScope,
		fn_type: Ty::Helper(1),
		deps: &[H::ReapFiber, H::TryFinalizeScope],
		build: |c| {
			task::build_cancel_scope_fn(c.dep(H::ReapFiber), c.dep(H::TryFinalizeScope), c.rt.taskg)
		},
	},
	HelperDef {
		id: H::ReapFiber,
		fn_type: Ty::Helper(1),
		deps: &[H::CancelScope, H::PollDefersState],
		build: |c| {
			task::build_reap_fiber_fn(
				c.dep(H::CancelScope),
				c.dep(H::PollDefersState),
				c.rt.net,
				c.rt.taskg,
			)
		},
	},
	HelperDef {
		id: H::TryFinalizeScope,
		fn_type: Ty::Helper(1),
		deps: &[H::ListAppend],
		build: |c| task::build_try_finalize_scope_fn(c.dep(H::ListAppend), c.rt.taskg, c.rt.tasklits),
	},
	HelperDef {
		id: H::Park,
		fn_type: Ty::Helper(3),
		deps: &[H::ListAppend],
		build: |c| task::build_park_fn(c.dep(H::ListAppend), c.rt.taskg),
	},
	HelperDef {
		id: H::ListAppend,
		fn_type: Ty::Helper(2),
		deps: &[H::ListPush],
		build: |c| task::build_list_append_fn(c.dep(H::ListPush)),
	},
	HelperDef {
		id: H::DrainNext,
		fn_type: Ty::Helper(1),
		deps: &[],
		build: |c| task::build_drain_next_fn(c.rt.taskg, c.rt.tasklits),
	},
	HelperDef {
		id: H::RunTimers,
		fn_type: Ty::Helper(0),
		deps: &[H::ListAppend],
		build: |c| task::build_run_timers_fn(c.dep(H::ListAppend), c.rt.taskg),
	},
	HelperDef {
		id: H::SchedCancel,
		fn_type: Ty::Helper(2),
		deps: &[H::ListAppend],
		build: |c| task::build_sched_cancel_fn(c.dep(H::ListAppend), c.rt.taskg),
	},
	HelperDef {
		id: H::SchedCancelAfter,
		fn_type: Ty::Helper(2),
		deps: &[H::ListAppend],
		build: |c| task::build_sched_cancel_after_fn(c.dep(H::ListAppend), c.rt.taskg),
	},
	// --- marshalling boundary (wasm↔host scratch memory) ---
	HelperDef {
		id: H::MarshalAlloc,
		fn_type: Ty::MarshalAlloc,
		deps: &[],
		build: |c| marshal::build_alloc_fn(c.rt.bump),
	},
	HelperDef {
		id: H::MarshalStore,
		fn_type: Ty::MarshalStore,
		deps: &[],
		build: |_| marshal::build_store_bytes_fn(),
	},
	HelperDef {
		id: H::MarshalLoad,
		fn_type: Ty::MarshalLoad,
		deps: &[],
		build: |_| marshal::build_load_bytes_fn(),
	},
	HelperDef {
		id: H::MarshalSend,
		fn_type: Ty::MarshalSend,
		deps: &[H::MarshalAlloc, H::MarshalStore],
		build: |c| {
			marshal::build_send_bytes_fn(c.rt.bump, c.dep(H::MarshalAlloc), c.dep(H::MarshalStore))
		},
	},
	HelperDef {
		id: H::MarshalReadNames,
		fn_type: Ty::MarshalReadNames,
		deps: &[H::MarshalLoad],
		build: |c| marshal::build_read_names_fn(c.dep(H::MarshalLoad)),
	},
	HelperDef {
		id: H::EntryError,
		fn_type: Ty::EntryError,
		deps: &[H::ToString, H::MarshalSend],
		build: |c| marshal::build_entry_error_fn(c.dep(H::ToString), c.dep(H::MarshalSend)),
	},
	HelperDef {
		id: H::DomRegister,
		// `(value) -> i32` — the same shape as `EntryError`.
		fn_type: Ty::EntryError,
		deps: &[H::ListPush],
		build: |c| {
			dom::build_dom_register_fn(
				c.rt
					.dom_handlers
					.expect("__dom_register needs the dom_handlers global"),
				c.dep(H::ListPush),
			)
		},
	},
	HelperDef {
		id: H::DomDispatch,
		fn_type: Ty::DomDispatch,
		deps: &[],
		build: |c| {
			let arity1 = c.arity(1);
			dom::build_dom_dispatch_fn(
				c.rt
					.dom_handlers
					.expect("__dom_dispatch needs the dom_handlers global"),
				arity1,
				// Browser MVU command-pump tail (wired in Phase 2 via `Helper::BrowserRun`).
				None,
			)
		},
	},
];

/// The helper a builtin tag lowers to, if any. These are the builtins implemented
/// as a synthetic wasm helper rather than a host import or an inline leaf — the
/// rest (`is_inline_builtin`, host imports) are classified in `Module::build`.
pub(crate) fn helper_for_tag(tag: &str) -> Option<Helper> {
	Some(match tag {
		"to-string" => H::ToString,
		// Higher-order builders: synthetic wasm helpers (loop + closure call).
		"list-build" => H::ListBuild,
		"list-collect" => H::ListCollect,
		"list-push" => H::ListPush,
		"bytes-build" => H::BytesBuild,
		// `bytes.concat` reuses the `__bytesconcat` helper.
		"bytes-concat" => H::BytesConcat,
		// dict table ops (see `helpers/dict.rs`): construct / mutate / probe the
		// mutable open-addressing table.
		"dict-empty" => H::DictEmpty,
		"dict-insert" => H::DictInsert,
		"dict-lookup" => H::DictLookup,
		"dict-remove" => H::DictRemove,
		"dict-map" => H::DictMap,
		"dict-filter" => H::DictFilter,
		"dict-size" => H::DictSize,
		"dict-entries" => H::DictEntries,
		"dict-update" => H::DictUpdate,
		"dict-clear" => H::DictClear,
		"dict-from-entries" => H::DictFromEntries,
		// Transient in-place insert + its owner token, emitted by `ir::reuse`.
		"dict-mint-token" => H::DictMintToken,
		"dict-insert-into" => H::DictInsertInto,
		// `wire-fingerprint` walks the schema value tree; encode/decode interpret
		// it to (de)serialize a value over the module-level codec globals.
		"wire-fingerprint" => H::WireFp,
		"wire-encode" => H::WireEnc,
		"wire-decode" => H::WireDec,
		_ => return None,
	})
}

/// Expand `req` to include every transitive dependency, so once a helper is in the
/// set, all the helpers its builder will reference are too.
pub(crate) fn close_deps(req: &mut HelperSet) {
	let mut stack: Vec<Helper> = req.iter().copied().collect();
	while let Some(h) = stack.pop() {
		for &d in REGISTRY[h as usize].deps {
			if req.insert(d) {
				stack.push(d);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registry_is_in_helper_order() {
		assert_eq!(REGISTRY.len(), Helper::COUNT);
		for (i, def) in REGISTRY.iter().enumerate() {
			assert_eq!(def.id as usize, i, "REGISTRY[{i}] is out of Helper order");
		}
	}
}
