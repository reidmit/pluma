// Runtime-helper bookkeeping: the catalog of synthetic `__*` helpers (`Helper`),
// which ones a reachable program needs (`scan_helpers` -> `HelperSet`), the wasm
// indices it resolves them to (`Runtime`/`HelperIndices`), the per-enum literal
// tables the codecs/formatters dispatch on, the realized lazy-global slots, and
// the host-vs-inline classification of builtin tags. The per-helper knowledge
// (type, deps, builder) lives in `helpers::REGISTRY`, walked in `Helper` order.

use std::collections::HashSet;

use ir::{Block, Rvalue, StmtKind};

use crate::types::FuncTypes;

/// The `$task` `kind` discriminants — the `TaskRepr` cases the async driver
/// dispatches on (`helpers/task.rs`). The async-fn lowering and the task-primitive
/// builtins build `$task`s with these; the driver reads them back.
pub(crate) mod task_kind {
	pub(crate) const PURE: i32 = 0;
	pub(crate) const FAIL: i32 = 1;
	pub(crate) const YIELD: i32 = 2;
	pub(crate) const SLEEP: i32 = 3;
	pub(crate) const THEN: i32 = 4;
	pub(crate) const ORELSE: i32 = 5;
	pub(crate) const ATTEMPT: i32 = 6;
	pub(crate) const MAP: i32 = 7;
	pub(crate) const ASYNC: i32 = 8;
	pub(crate) const SHIELDED: i32 = 9;
	// Structured-concurrency kinds — used by the Stage 2 scheduler.
	#[allow(dead_code)]
	pub(crate) const SCOPE: i32 = 10;
	#[allow(dead_code)]
	pub(crate) const HANDLE: i32 = 11;
	#[allow(dead_code)]
	pub(crate) const NEXT: i32 = 12;
}

/// Activation kinds — an entry in a fiber's await chain (the driver's activation
/// stack). Encoded as a `$variant` with this as its `vtag` and `[x, y]` payload.
/// Mirrors `vm::task::Activation` (minus `Async`: the wasm driver is poll-only).
pub(crate) mod act_kind {
	pub(crate) const POLL: i32 = 0; // (poll_closure, state)
	pub(crate) const THEN: i32 = 1; // (k)
	pub(crate) const ORELSE: i32 = 2; // (recover)
	pub(crate) const ATTEMPT: i32 = 3; // ()
	pub(crate) const MAP: i32 = 4; // (f)
}

/// The Stage-2 cooperative scheduler's layout constants — fiber/scope field
/// indices (each is a mutable `$valarray` "record"), and the small kind enums the
/// scheduler encodes as boxed ints. Mirrors `vm::task`'s `Fiber`/`Scope`/`Wait`/
/// `Outcome`/`Focus`.
pub(crate) mod sched {
	/// `Fiber` fields (a mutable `$valarray` of `COUNT` boxed slots).
	pub(crate) mod fiber {
		pub(crate) const ACT: u32 = 0; // $list of activation $variants (the await chain)
		pub(crate) const SCOPE: u32 = 1; // boxed int — owning scope id
		pub(crate) const RUNS_SCOPE: u32 = 2; // boxed int — scope id this is the body of (-1 = none)
		pub(crate) const RES_KIND: u32 = 3; // boxed int — outcome kind (0 none/1 ok/2 err/3 cancelled)
		pub(crate) const RES_VAL: u32 = 4; // value — settled result
		pub(crate) const WAIT_KIND: u32 = 5; // boxed int — what it's parked on
		pub(crate) const WAIT_ARG: u32 = 6; // boxed int — the park target (fid/sid)
		pub(crate) const ALIVE: u32 = 7; // boxed int — 0/1
		pub(crate) const WAITERS: u32 = 8; // $list of waiter fids (boxed ints)
		pub(crate) const COUNT: u32 = 9;
	}
	/// `Scope` fields.
	pub(crate) mod scope {
		pub(crate) const MANUAL: u32 = 0; // boxed int — 0/1
		pub(crate) const CANCELLED: u32 = 1; // boxed int — 0/1
		pub(crate) const FINALIZED: u32 = 2; // boxed int — 0/1
		pub(crate) const BODY: u32 = 3; // boxed int — root body fid
		pub(crate) const CHILDREN: u32 = 4; // $list of child fids
		pub(crate) const AWAITER: u32 = 5; // boxed int — fid awaiting this scope (-1 = none)
		pub(crate) const BD_KIND: u32 = 6; // boxed int — body outcome kind (0 = not done)
		pub(crate) const BD_VAL: u32 = 7; // value — body outcome value
		pub(crate) const FAIL_SET: u32 = 8; // boxed int — 0/1 (a fail-fast failure is set)
		pub(crate) const FAIL_VAL: u32 = 9; // value — the failure
		pub(crate) const COMPLETED: u32 = 10; // $list of settled child outcomes (for s.next), FIFO
		pub(crate) const NEXT_WAITERS: u32 = 11; // $list of fids parked in s.next
		pub(crate) const COUNT: u32 = 12;
	}
	/// What a fiber is parked on (`Wait`).
	pub(crate) mod wait {
		pub(crate) const NONE: i32 = 0;
		pub(crate) const YIELD: i32 = 1;
		pub(crate) const SLEEP: i32 = 2; // arg = nanos
		pub(crate) const HANDLE: i32 = 3; // arg = child fid
		pub(crate) const NEXT: i32 = 4; // arg = scope id
		pub(crate) const SCOPE: i32 = 5; // arg = scope id
	}
	/// A fiber's focus on its next turn (`Focus`).
	pub(crate) mod focus {
		pub(crate) const START: i32 = 0;
		pub(crate) const OK: i32 = 1;
		pub(crate) const ERR: i32 = 2;
	}
	/// How a fiber/scope finished (`Outcome`).
	pub(crate) mod outcome {
		pub(crate) const NONE: i32 = 0;
		pub(crate) const OK: i32 = 1;
		pub(crate) const ERR: i32 = 2;
		pub(crate) const CANCELLED: i32 = 3;
	}
	pub(crate) const NO_AWAITER: i64 = -1;
	pub(crate) const NO_SCOPE: i64 = -1;
	pub(crate) const ROOT_SCOPE: i64 = 0;
}

/// A reachable IR global realized as a lazily-initialized wasm value: a cached
/// value (`val_idx`) behind an `i32` init flag (`init_idx`), built on first
/// access. (Builtin globals are call-only; Const globals aren't realized yet.)
#[derive(Clone)]
pub(crate) struct GlobalSlot {
	pub(crate) val_idx: u32,
	pub(crate) init_idx: u32,
	pub(crate) kind: GlobalKind,
}

#[derive(Clone)]
pub(crate) enum GlobalKind {
	/// A top-level def: run its thunk (wasm index) once.
	Thunk(u32),
	/// A trait-instance method dict: build a `$methoddict` of builtin-wrapper
	/// closures (each method's wrapper wasm index).
	MethodDict(Vec<u32>),
}

/// Every synthetic `__*` runtime helper. The variant order is the contract: both
/// index allocation and emission walk `helpers::REGISTRY` (which is in this same
/// order), so a helper's position here is its emission slot. Adding a helper is
/// one `REGISTRY` row plus its builder — see `helpers/mod.rs`.
///
/// What each helper is:
/// - `Eq` — `__eq(value, value) -> i32` structural equality.
/// - `GetField`/`RecordUpdate` — record field read / one-field copy (both via `__eq`).
/// - `ListTail` — the `...rest` tail of a list pattern.
/// - `ArrConcat`/`BytesConcat` — value-array / byte-array concat (spread, `++`, interp).
/// - `ToString`/`IntStr` — `vm::Value`'s `Display` in wasm + its decimal-int helper.
/// - `ListBuild`/`ListCollect`/`BytesBuild` — tabulating builders (`[f 0, …, f (n-1)]`).
/// - `Dict*` — insert/lookup/remove/map/filter over the `$dict` entries array.
/// - `WireFp`/`WireMixStr`/`WireMixLen` — the `wire` FNV fingerprint + its mixers.
/// - `Wire*` (the codec) — the `wire-encode`/`wire-decode` machinery over the
///   module-level scratch globals (`WireGlobals`): `WirePush`/`WireUvarint` are
///   the encode byte/varint sinks, `WireEnc` the recursive encoder, `WireRByte`/
///   `WireRUvarint` the decode byte/varint sources, `WireDec`/`WireDecVariant`
///   the recursive decoder, `WireCtxPut`/`WireCtxGet` the recursive-enum
///   registry, `WireDisp` rebuilds a decoded variant's display name, and
///   `WireResult` wraps a decoded value in `ok`/`err` (the trailing-bytes check).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Helper {
	Eq,
	GetField,
	RecordUpdate,
	ListTail,
	ArrConcat,
	BytesConcat,
	ToString,
	IntStr,
	ListBuild,
	ListCollect,
	BytesBuild,
	DictInsert,
	DictLookup,
	DictRemove,
	DictMap,
	DictFilter,
	WireFp,
	WireMixStr,
	WireMixLen,
	WirePush,
	WireUvarint,
	WireCtxPut,
	WireCtxGet,
	WireEnc,
	WireEncVariant,
	WireRByte,
	WireRUvarint,
	WireDisp,
	WireDecVariant,
	WireDec,
	WireResult,
	WireBCmp,
	WireEncDict,
	/// `__record_rest(rec, excluded)` — build the uniform `$record` of `rec`'s
	/// fields minus the names in the `excluded` `$list`. Backs `...rest` on a
	/// *uniform* match subject (the nominal path builds the rest inline).
	RecordRest,
	/// `__run_defers(defers)` — run a function's scheduled `defer` cleanups LIFO
	/// at exit. `defers` is a `$list` of zero-arg cleanup closures kept in
	/// last-pushed-first order (the emitter prepends), so this calls them front
	/// to back. Returns `nothing`. Backs sync `defer` (the async path runs its
	/// cleanups through the CPS poll driver instead).
	RunDefers,
	// --- async runtime (the hand-emitted task/scope driver, `helpers/task.rs`) ---
	/// `__task_drive(root) -> value` — the single-fiber poll driver. Runs a cold
	/// `$task` to completion (mirroring `vm::task::advance_one`'s Start/Ok/Err
	/// focus loop over an activation stack), returning the success value or a
	/// `result.err(e)` on root failure.
	TaskDrive,
	/// `__poll_step(poll_closure, state, resume) -> $tuple(kind, x, y)` — advance
	/// one CPS poll: call the poll fn, interpret its `__poll` (`ready`/`pending`),
	/// running any completion `defer`s. `kind` 0 = complete (x = tail task), 1 =
	/// pending (x = sub-task, y = next state).
	PollStep,
	/// `__poll_defers_list(list) -> nothing` — run a `$list` of zero-arg cleanup
	/// closures LIFO (the CPS pass appends, so back to front).
	PollDefersList,
	/// `__poll_defers_state(state) -> nothing` — run the `__defers` cleanup list
	/// carried in a suspended poll state (tolerant of its absence), on the
	/// failure/cancellation path.
	PollDefersState,
	/// `__act_push(activation) -> nothing` — push one activation `$value` onto the
	/// driver's global activation stack, growing it as needed.
	ActPush,
	/// `__task_entry(env) -> value` — the async program entry: call the real IR
	/// entry, then drive the task it returns. Exported as `_entry` when async.
	TaskEntry,
	// --- Stage 2 cooperative scheduler (helpers/task.rs) ---
	/// `__pump(fid, fkind, fval)` — advance one fiber until it completes or parks.
	Pump,
	/// `__start_scope(fid, manual, body_fn) -> sid` — create a scope + body fiber.
	StartScope,
	/// `__sched_spawn(handle, task) -> handle-task` — `s.spawn` (side-effecting).
	SchedSpawn,
	/// `__fiber_completed(fid, kind, val)` — route a settled fiber's outcome.
	FiberCompleted,
	/// `__on_body_done(sid, kind, val)` — a scope body settled.
	OnBodyDone,
	/// `__on_child_done(sid, fid, kind, val)` — a spawned child settled.
	OnChildDone,
	/// `__cancel_scope(sid)` — cancel a scope and everything it owns.
	CancelScope,
	/// `__reap_fiber(fid)` — abandon a fiber, running its `defer`s.
	ReapFiber,
	/// `__try_finalize_scope(sid)` — finalize once body + children have settled.
	TryFinalizeScope,
	/// `__park(fid, wait_kind, wait_arg)` — register a parked fiber.
	Park,
	/// `__list_append(list, elem) -> list` — append one element (O(n) rebuild).
	ListAppend,
}

impl Helper {
	/// Variant count; the discriminants are `0..COUNT`, used to index
	/// `HelperIndices`. A test in `helpers` checks `REGISTRY` stays this length
	/// and in-order.
	pub(crate) const COUNT: usize = 52;
}

/// The wasm index assigned to each emitted helper (`None` = not in the reachable
/// program). Indexed by `Helper as usize`; stays `Copy` so `Runtime` can be.
#[derive(Clone, Copy)]
pub(crate) struct HelperIndices([Option<u32>; Helper::COUNT]);

impl Default for HelperIndices {
	fn default() -> Self {
		Self([None; Helper::COUNT])
	}
}

impl HelperIndices {
	pub(crate) fn get(&self, h: Helper) -> Option<u32> {
		self.0[h as usize]
	}
	pub(crate) fn set(&mut self, h: Helper, idx: u32) {
		self.0[h as usize] = Some(idx);
	}
}

/// The helpers a reachable program needs — before and after dependency expansion
/// (see `helpers::close_deps`).
pub(crate) type HelperSet = HashSet<Helper>;

/// Resolved wasm state every function can reach: the synthetic-helper indices, the
/// `float_to_str` host import, and the per-enum literal tables the codecs and
/// formatters dispatch on. Stays `Copy` (every field is an index or POD).
#[derive(Clone, Copy, Default)]
pub(crate) struct Runtime {
	/// Wasm index of each emitted synthetic helper.
	pub(crate) helpers: HelperIndices,
	/// Host import `float_to_str(f64, $bytes buf) -> i32 len` — float formatting
	/// (delegated to the host, like a browser's `String(x)`), used by `__tostring`.
	pub(crate) float_to_str: Option<u32>,
	/// Data-segment offsets/lengths for the literal strings `__tostring` needs.
	pub(crate) lits: ToStringLits,
	/// `some`/`none` variant info for `__dict_lookup` to build its `option` result.
	pub(crate) opt: OptionLits,
	/// `lt`/`eq`/`gt` variant info for the `*-compare` wrappers' `ordering` result.
	pub(crate) ord: OrderingLits,
	/// The `wire-schema` enum's per-variant tags, for the codec helpers' dispatch.
	pub(crate) wire: WireTags,
	/// The module-level scratch globals the `wire` codec threads its recursive
	/// encode/decode state through (buffer, cursor, error, enum registry).
	pub(crate) wireg: WireGlobals,
	/// The `result` / `wire-error` variant tags + display names `__wire_result`
	/// builds when wrapping a decoded value in `ok`/`err`.
	pub(crate) wirelits: WireResultLits,
	/// Wasm index of the IR program entry (`main`), so `__task_entry` can call it
	/// before driving the task it returns. Set when the program is async.
	pub(crate) entry_idx: Option<u32>,
	/// The async driver's module-level scratch globals (the activation stack).
	pub(crate) taskg: TaskGlobals,
	/// `result` `ok`/`err` tags + display names (for `task.attempt` and root
	/// failure) and the `__defers` field name the driver scans for.
	pub(crate) tasklits: TaskLits,
}

impl Runtime {
	/// The wasm index of helper `h`, if the program emitted it.
	pub(crate) fn idx(&self, h: Helper) -> Option<u32> {
		self.helpers.get(h)
	}
}

/// One helper's wasm function type, resolved against the interner at emission.
/// Mirrors the `FuncTypes::for_*` constructors.
#[derive(Clone, Copy)]
pub(crate) enum Ty {
	Eq,
	Helper(usize),
	ArrConcat,
	BytesConcat,
	WireMixVal,
	WireMixLen,
	WirePush,
	WireUvarint,
	WireEnc,
	WireRByte,
	WireRUvarint,
}

impl Ty {
	pub(crate) fn resolve(self, ft: &mut FuncTypes) -> u32 {
		match self {
			Ty::Eq => ft.for_eq(),
			Ty::Helper(n) => ft.for_helper(n),
			Ty::ArrConcat => ft.for_arrconcat(),
			Ty::BytesConcat => ft.for_bytesconcat(),
			Ty::WireMixVal => ft.for_wire_mix_val(),
			Ty::WireMixLen => ft.for_wire_mix_len(),
			Ty::WirePush => ft.for_wire_push(),
			Ty::WireUvarint => ft.for_wire_uvarint(),
			Ty::WireEnc => ft.for_wire_enc(),
			Ty::WireRByte => ft.for_wire_rbyte(),
			Ty::WireRUvarint => ft.for_wire_ruvarint(),
		}
	}
}

/// What a helper builder is handed at emission: its own wasm index (for self-
/// recursion), the resolved `Runtime` (dependency indices + literal tables), and
/// the type interner (for the closure arity types the tabulating builders need).
pub(crate) struct HelperCtx<'a> {
	pub(crate) self_idx: u32,
	pub(crate) rt: &'a Runtime,
	pub(crate) ftypes: &'a mut FuncTypes,
}

impl HelperCtx<'_> {
	/// The wasm index of a declared dependency — always present, since
	/// `close_deps` pulls every dep into the program before allocation.
	pub(crate) fn dep(&self, h: Helper) -> u32 {
		self
			.rt
			.idx(h)
			.expect("a present helper's declared dependency is always allocated")
	}
	/// Intern the func type of an `n`-arg closure the builder will `call_indirect`.
	pub(crate) fn arity(&mut self, n: usize) -> u32 {
		self.ftypes.for_arity(n)
	}
	/// The `float_to_str` host import index (present whenever `ToString` is).
	pub(crate) fn float_to_str(&self) -> u32 {
		self.rt.float_to_str.expect("__tostring needs float_to_str")
	}
}

/// FNV-1a 64-bit offset basis / prime — the constants `vm::wire` mixes with, so
/// the wasm fingerprint matches the VM's byte-for-byte. `OFFSET` seeds the hash
/// (at the `wire-fingerprint` call site); `PRIME` is folded by the mixers.
pub(crate) const WIRE_FNV_OFFSET: i64 = 0xcbf2_9ce4_8422_2325u64 as i64;
pub(crate) const WIRE_FNV_PRIME: i64 = 0x0000_0100_0000_01b3;

/// The within-enum tags of the `wire-schema` prelude enum's variants, resolved
/// from the enum table so the codec helpers can dispatch on a schema node's
/// runtime `vtag` rather than its name string.
#[derive(Clone, Copy, Default)]
pub(crate) struct WireTags {
	pub(crate) s_int: u32,
	pub(crate) s_float: u32,
	pub(crate) s_bool: u32,
	pub(crate) s_string: u32,
	pub(crate) s_bytes: u32,
	pub(crate) s_duration: u32,
	pub(crate) s_nothing: u32,
	pub(crate) s_list: u32,
	pub(crate) s_dict: u32,
	pub(crate) s_enum_ref: u32,
	pub(crate) s_tuple: u32,
	pub(crate) s_record: u32,
	pub(crate) s_enum: u32,
}

/// The wasm indices of the module-level mutable globals the `wire` codec uses as
/// scratch state. Encode writes into `buf`/`len` (a doubling byte buffer);
/// decode reads from `in`/`pos` and reports failure through `err`/`errval`; both
/// thread the recursive-enum registry through `ctx`/`ctxlen` (a `$valarray` of
/// `$tuple(qualified-name $str, variants $list)` entries). Allocated only when a
/// reachable program calls `wire-encode`/`wire-decode`. Codes in `err`: 0=ok,
/// 1=unexpected-end, 2=invalid-tag (`errval`=tag), 3=invalid-utf8,
/// 4=trailing-bytes (`errval`=count), 5=malformed.
#[derive(Clone, Copy, Default)]
pub(crate) struct WireGlobals {
	pub(crate) buf: u32,    // mut ref null $bytes — encode output
	pub(crate) len: u32,    // mut i32 — encode used length
	pub(crate) input: u32,  // mut ref null $bytes — decode input
	pub(crate) pos: u32,    // mut i32 — decode cursor
	pub(crate) err: u32,    // mut i32 — decode error code
	pub(crate) errval: u32, // mut i64 — decode error payload
	pub(crate) ctx: u32,    // mut ref null $valarray — enum-ctx registry
	pub(crate) ctxlen: u32, // mut i32 — registry used length
}

/// The `result`/`wire-error` variant tags + interned display-name `(off, len)`
/// strings `__wire_result` needs to wrap a decoded value: `ok v` on success, or
/// the `wire-error` variant matching the codec's error code on failure.
#[derive(Clone, Copy, Default)]
pub(crate) struct WireResultLits {
	pub(crate) ok_tag: u32,
	pub(crate) err_tag: u32,
	pub(crate) ok_name: (u32, u32),
	pub(crate) err_name: (u32, u32),
	/// `(tag, display-name)` for each `wire-error` variant, indexed by error code
	/// minus one: `[unexpected-end, invalid-tag, invalid-utf8, trailing-bytes,
	/// malformed]`.
	pub(crate) errors: [(u32, (u32, u32)); 5],
}

/// What an `*-compare` wrapper needs to construct an `ordering` `$variant`: each
/// variant's within-enum tag and its interned display-name string `(off, len)`.
#[derive(Clone, Copy, Default)]
pub(crate) struct OrderingLits {
	pub(crate) lt_tag: u32,
	pub(crate) eq_tag: u32,
	pub(crate) gt_tag: u32,
	pub(crate) lt_name: (u32, u32),
	pub(crate) eq_name: (u32, u32),
	pub(crate) gt_name: (u32, u32),
}

/// The async scheduler's module-level mutable globals. The currently-pumping
/// fiber's await chain is loaded into `act`/`actlen` (a growable `$valarray`
/// stack) for the duration of its pump, then saved back to its `Fiber.ACT`. The
/// rest is the cooperative scheduler state (`vm::task::Scheduler`): the fiber and
/// scope tables, the ready deque, the virtual timer list, and the deferred-cancel
/// queue, plus the pump's outcome/park output channel. Allocated only when async.
#[derive(Clone, Copy, Default)]
pub(crate) struct TaskGlobals {
	pub(crate) act: u32,    // mut ref null $valarray — current fiber's activation stack
	pub(crate) actlen: u32, // mut i32 — activation count
	pub(crate) fibers: u32, // mut ref null $value — $list of fiber field-arrays (by fid)
	pub(crate) scopes: u32, // mut ref null $value — $list of scope field-arrays (by sid)
	pub(crate) ready: u32,  // mut ref null $value — $list of ready entries (fid, focus_kind, val)
	pub(crate) rhead: u32,  // mut i32 — ready deque head cursor (pop_front)
	pub(crate) timers: u32, // mut ref null $value — $list of timer entries (at, kind, arg)
	pub(crate) pending: u32, // mut ref null $value — $list of scope ids to cancel between steps
	pub(crate) now: u32,    // mut i64 — virtual clock (nanoseconds)
	pub(crate) root_kind: u32, // mut i32 — root outcome kind (0 = not done yet)
	pub(crate) root_val: u32,  // mut ref null $value — root outcome value
	pub(crate) out_kind: u32,  // mut i32 — pump output: 1 done / 2 park
	pub(crate) out_okerr: u32, // mut i32 — on done: outcome kind (ok/err); on park: wait kind
	pub(crate) out_val: u32,   // mut ref null $value — on done: outcome value
	pub(crate) out_arg: u32,   // mut i32 — on park: wait arg (fid/sid), or sleep nanos low bits unused
	pub(crate) out_arg64: u32, // mut i64 — on park sleep: nanos
}

/// What the async driver needs to build `result`/`option` variants and find a
/// poll state's cleanup list: the `ok`/`err`/`some`/`none` tags + interned display
/// names, the interned `__defers` field name, and the "scope cancelled" error
/// string a self-cancelled scope hands its awaiter. `(off, len)` are data-segment
/// offsets.
#[derive(Clone, Copy, Default)]
pub(crate) struct TaskLits {
	pub(crate) ok_tag: u32,
	pub(crate) err_tag: u32,
	pub(crate) ok_name: (u32, u32),
	pub(crate) err_name: (u32, u32),
	pub(crate) some_tag: u32,
	pub(crate) none_tag: u32,
	pub(crate) some_name: (u32, u32),
	pub(crate) none_name: (u32, u32),
	pub(crate) defers_name: (u32, u32),
	pub(crate) cancelled_msg: (u32, u32),
}

/// What `__dict_lookup` needs to construct `some v` / `none` `$variant`s: each
/// variant's within-enum tag and its interned display-name string `(off, len)`.
#[derive(Clone, Copy, Default)]
pub(crate) struct OptionLits {
	pub(crate) some_tag: u32,
	pub(crate) none_tag: u32,
	pub(crate) some_name: (u32, u32),
	pub(crate) none_name: (u32, u32),
}

/// `(offset, len)` of each fixed literal `__tostring` emits, in the shared data
/// segment.
#[derive(Clone, Copy, Default)]
pub(crate) struct ToStringLits {
	pub(crate) unit: (u32, u32),
	pub(crate) tru: (u32, u32),
	pub(crate) fals: (u32, u32),
	pub(crate) lparen: (u32, u32),
	pub(crate) rparen: (u32, u32),
	pub(crate) lbrack: (u32, u32),
	pub(crate) rbrack: (u32, u32),
	pub(crate) lbrace: (u32, u32),
	pub(crate) rbrace: (u32, u32),
	pub(crate) comma_sp: (u32, u32), // ", "
	pub(crate) colon_sp: (u32, u32), // ": "
	pub(crate) space: (u32, u32),    // " "
	pub(crate) ref_pfx: (u32, u32),  // "ref "
}

/// Collect the helpers an IR `Block` needs by *construct* — the ones triggered by
/// syntax (`==`, field access, list spread, `++`/interpolation, list-rest
/// patterns) rather than by a named builtin call (those are added in
/// `Module::build`). Transitive dependencies (e.g. `GetField` -> `Eq`) are filled
/// in afterwards by `helpers::close_deps`, so this only records direct triggers.
pub(crate) fn scan_helpers(b: &Block, req: &mut HelperSet) {
	fn rv(rv: &Rvalue, req: &mut HelperSet) {
		match rv {
			Rvalue::Bin(ir::BinOp::Eq | ir::BinOp::Ne, _, _) => {
				req.insert(Helper::Eq);
			}
			Rvalue::GetField(..) => {
				req.insert(Helper::GetField);
			}
			Rvalue::RecordUpdate { .. } => {
				req.insert(Helper::RecordUpdate);
			}
			Rvalue::MakeList(items) => {
				if items.iter().any(|it| matches!(it, ir::ListItem::Spread(_))) {
					req.insert(Helper::ArrConcat);
				}
			}
			Rvalue::Bin(ir::BinOp::Concat, _, _) | Rvalue::Interpolate(_) => {
				req.insert(Helper::BytesConcat);
			}
			_ => {}
		}
	}
	fn pat(p: &ir::Pattern, req: &mut HelperSet) {
		match p {
			ir::Pattern::List {
				rest: Some(ir::ListRest::Bind(_)),
				items,
			} => {
				req.insert(Helper::ListTail);
				items.iter().for_each(|p| pat(p, req));
			}
			ir::Pattern::List { items, .. } => items.iter().for_each(|p| pat(p, req)),
			ir::Pattern::Variant { fields, .. } | ir::Pattern::Tuple(fields) => {
				fields.iter().for_each(|p| pat(p, req))
			}
			ir::Pattern::Record { fields, rest, .. } => {
				// Record patterns match fields via `__getfield` (which uses `__eq`).
				req.insert(Helper::GetField);
				// A `...rest` on a uniform subject filters via `__record_rest`. (A
				// nominal subject builds the rest inline; the request is conservative
				// since nominality is an emit-time decision.)
				if matches!(rest, ir::RecordRest::Bind(_)) {
					req.insert(Helper::RecordRest);
				}
				fields.iter().for_each(|(_, p)| pat(p, req));
			}
			// String/bytes literal patterns match via structural `__eq`.
			ir::Pattern::Literal(ir::Const::Str(_) | ir::Const::Bytes(_)) => {
				req.insert(Helper::Eq);
			}
			_ => {}
		}
	}
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, r) | StmtKind::Discard(r) => rv(r, req),
			StmtKind::If(_, t, e) => {
				scan_helpers(t, req);
				scan_helpers(e, req);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					scan_helpers(b, req);
				}
				scan_helpers(default, req);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					pat(&a.pattern, req);
					scan_helpers(&a.body, req);
				}
			}
			StmtKind::Loop(b) => scan_helpers(b, req),
			// `defer expr` prepends a cleanup closure (`__arrconcat` over the
			// accumulator `$list`) and runs the list at every `Return`
			// (`__run_defers`).
			StmtKind::PushDefer(_) => {
				req.insert(Helper::ArrConcat);
				req.insert(Helper::RunDefers);
			}
			_ => {}
		}
	}
}

/// A host primitive's calling shape: how many boxed args it takes, and whether it
/// returns a boxed value (vs. nothing — in which case the caller materializes the
/// Pluma `nothing` result).
pub(crate) struct HostSig {
	pub(crate) arity: usize,
	pub(crate) returns_value: bool,
}

/// The host signature for a builtin tag, or `None` if this backend doesn't yet
/// import it. Grows with milestone coverage (M7 brings the rest).
pub(crate) fn host_sig(tag: &str) -> Option<HostSig> {
	match tag {
		// stdout/stderr writers + the program-controlled abort. All take one
		// boxed arg and return nothing (`io.fail` diverges — the host traps).
		"print" | "io-print" | "io-print-err" | "io-write" | "io-write-err" | "io-write-bytes"
		| "io-write-err-bytes" | "io-fail" => Some(HostSig {
			arity: 1,
			returns_value: false,
		}),
		_ => None,
	}
}

/// Pure-compute builtins emitted inline at the call site (no host import, no
/// synthetic helper). They operate on the GC `$value` layout directly. Grows as
/// more of the builtin surface moves to native WasmGC.
pub(crate) fn is_inline_builtin(tag: &str) -> bool {
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
			// duration's nanosecond count: a retag of the `$int`-shaped box.
			| "time-duration-as-nanos"
			// duration / instant box+unbox. Both reuse the `$int` shape
			// (`{tag, i64}`); these retag between `TAG_INT` and the carrier tag.
			| "time-duration-of-nanos"
			| "time-from-unix-nanos"
			| "time-to-unix-nanos"
	)
}

/// The `$task` `kind` a `task.*`/`scope-*` *pure constructor* builtin builds, if
/// any. These need no host import (they build a `$task` inline) and no `__poll`
/// driver at the call site — the scheduler runs them later. The side-effecting
/// scope-kernel ops (`scope-spawn`/`scope-cancel`/`scope-cancel-after`) are NOT
/// here — they're routed to driver helpers in `emit.rs`.
pub(crate) fn task_builtin_kind(tag: &str) -> Option<i32> {
	Some(match tag {
		"task-return" => task_kind::PURE,
		"task-fail" => task_kind::FAIL,
		"task-yield" => task_kind::YIELD,
		"task-sleep" => task_kind::SLEEP,
		"task-then" => task_kind::THEN,
		"task-or-else" => task_kind::ORELSE,
		"task-attempt" => task_kind::ATTEMPT,
		"task-map" => task_kind::MAP,
		"task-shielded" => task_kind::SHIELDED,
		"scope-new" => task_kind::SCOPE,
		"scope-next" => task_kind::NEXT,
		_ => return None,
	})
}

/// Transcendental float math with no WasmGC opcode (log/log10/log2/exp/sin/cos).
/// Each is a `(f64) -> f64` host import (the same libm call the VM makes); the
/// `$float` box/unbox is emitted in wasm around the call.
pub(crate) fn is_f64_unary_host(tag: &str) -> bool {
	matches!(
		tag,
		"math-log" | "math-log10" | "math-log2" | "math-exp" | "math-sin" | "math-cos"
	)
}
