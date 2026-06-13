// Runtime-helper bookkeeping: the catalog of synthetic `__*` helpers (`Helper`),
// which ones a reachable program needs (`scan_helpers` -> `HelperSet`), the wasm
// indices it resolves them to (`Runtime`/`HelperIndices`), the per-enum literal
// tables the codecs/formatters dispatch on, the realized lazy-global slots, and
// the host-vs-inline classification of builtin tags. The per-helper knowledge
// (type, deps, builder) lives in `helpers::REGISTRY`, walked in `Helper` order.

use crate::types::FuncTypes;
use ir::{Block, Rvalue, StmtKind};
use std::collections::HashSet;

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
	// std/sys/net suspending socket ops — dispatched like any other task kind, but
	// each does a non-blocking host call and parks the fiber on the reactor
	// (`wait::IO`, re-run via `fiber::RETRY`) when it would block. See
	// `helpers/task.rs`'s pump NET arms + block step.
	pub(crate) const NET_ACCEPT: i32 = 13;
	pub(crate) const NET_READ: i32 = 14;
	pub(crate) const NET_WRITE: i32 = 15;
	// `rpc-stream-next` (`std/web/stream`): pull the next event off a host-fed RPC
	// stream channel, or park (`wait::RPC`) until the host pushes one via
	// `__rpc_stream_event`. The browser's push analogue of the net read's pull park.
	pub(crate) const RPC_NEXT: i32 = 16;
	// `web-fetch` in the browser (`std/web/fetch`): the unary single-shot case of
	// `RPC_NEXT`. The task carries a channel token; the pump pulls the one reply the
	// host pushes and shapes it into a `result string string` (`ok <reply>` / `err`),
	// or parks (`wait::RPC`) until the host's async `fetch` delivers. The sys host
	// lowers `web-fetch` to a blocking exchange instead (no task kind).
	pub(crate) const WEB_FETCH: i32 = 17;
	// BlockingPool offload ops (host/src/offload.rs): a non-pollable blocking call (`offload-sleep`
	// in v0; async-fs read/write next) submitted to a host worker thread, parking the fiber
	// on `wait::IO` and woken through `io-poll` — the completion analogue of the net read's
	// readiness park. Settled by the same `io_settle` shape (`ok …`/`err …`).
	pub(crate) const OFFLOAD_SLEEP: i32 = 18;
	// std/sys/fs (host/src/offload.rs): one generic op (`fs-op`) for the whole async surface. The
	// op-code selects read/write/stat/remove/… host-side; the pump marshals op + path + data
	// in and bytes out (with the `io-copyout` overflow path — payload size is unknown), and
	// the Pluma wrapper interprets the bytes per op. Parks on `wait::IO` while a pool worker
	// runs the blocking `std::fs` call.
	pub(crate) const FILE_OP: i32 = 19;
	// `net.connect`: dial a server, offloaded to a pool worker so the blocking DNS + TCP
	// handshake don't stall the scheduler (host/src/offload.rs). Unlike accept/read/write (which
	// park on socket *readiness*), connect parks on offload *completion* — the worker hands
	// back the connected socket, adopted into the table on collect. Settles `result conn`.
	pub(crate) const NET_CONNECT: i32 = 20;
	// std/sys/db (host/src/db.rs): one generic op (`db-op`) for open/execute/close, selected by
	// an op-code, offloaded to the pinned SQLite worker. The pump marshals op-code + connection
	// id + sql/path + the encoded params blob in and bytes out (rows, or a new connection id as
	// text — `io-copyout` overflow path, payload size unknown). Parks on `wait::IO` like fs.
	pub(crate) const DB_OP: i32 = 21;
}

/// The host-fed RPC stream channel (`std/web/stream`): a per-subscription mailbox
/// the browser loader pushes SSE events into (`__rpc_stream_event`) and a parked
/// `rpc-stream-next` fiber drains. A channel is a `$tuple(TAG_TUPLE, $valarray)`
/// stored in the `rpc_channels` registry `$list` (same record shape as the
/// fiber/scope tables), keyed by the token `rpc-stream-open` hands the host.
pub(crate) mod rpc_chan {
	/// `$list` of `$bytes` — pending `next` payloads (the wire-encoded elements),
	/// FIFO. Appended by `__rpc_stream_event`, drained by `rpc-stream-next` via HEAD.
	pub(crate) const QUEUE: u32 = 0;
	/// boxed int — index of the next unread QUEUE element (a head cursor, so a
	/// dequeue is O(1) and never rebuilds the list).
	pub(crate) const HEAD: u32 = 1;
	/// boxed int — fid of the fiber parked in `rpc-stream-next` on this channel, or
	/// -1 if none is waiting. The host wake reads it to re-ready that one fiber.
	pub(crate) const WAITER: u32 = 2;
	/// boxed int — 0/1: the host sent the terminal `done` event (clean end).
	pub(crate) const DONE: u32 = 3;
	/// boxed int — 0/1: the host sent a `fault` event (the stream errored).
	pub(crate) const FAULTED: u32 = 4;
	pub(crate) const COUNT: u32 = 5;

	/// The event kind the host passes to `__rpc_stream_event` — the SSE event name,
	/// pre-classified host-side (the loader parses the text framing).
	pub(crate) const EV_NEXT: i32 = 0;
	pub(crate) const EV_DONE: i32 = 1;
	pub(crate) const EV_FAULT: i32 = 2;
}

/// Activation kinds — an entry in a fiber's await chain (the driver's activation
/// stack). Encoded as a `$variant` with this as its `vtag` and `[x, y]` payload.
/// (No `Async` activation: the wasm driver is poll-only.)
pub(crate) mod act_kind {
	pub(crate) const POLL: i32 = 0; // (poll_closure, state)
	pub(crate) const THEN: i32 = 1; // (k)
	pub(crate) const ORELSE: i32 = 2; // (recover)
	pub(crate) const ATTEMPT: i32 = 3; // ()
	pub(crate) const MAP: i32 = 4; // (f)
	pub(crate) const SHIELD: i32 = 5; // () — marks a shielded region's end on the chain
}

/// The Stage-2 cooperative scheduler's layout constants — fiber/scope field
/// indices (each is a mutable `$valarray` "record"), and the small kind enums the
/// scheduler encodes as boxed ints — the `Fiber`/`Scope`/`Wait`/`Outcome`/`Focus`
/// layout the scheduler dispatches on.
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
		pub(crate) const RETRY: u32 = 9; // value — parked net `$task` re-Started on socket readiness (wait::IO)
		pub(crate) const ENV: u32 = 10; // value — task-local binding env: a cons-chain of `[cell, val, next]` `$tuple`s (null = empty). Captured parent→child at spawn; read by `local-get`.
		pub(crate) const COUNT: u32 = 11;
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
		pub(crate) const IO: i32 = 6; // parked on socket readiness; the reactor re-runs `fiber::RETRY`
		pub(crate) const RPC: i32 = 7; // arg = channel token; the host re-runs `fiber::RETRY` on `__rpc_stream_event`
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
/// - `ToString`/`IntStr` — canonical `to-string` formatting in wasm + its decimal-int helper.
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
	ListPush,
	BytesBuild,
	DictInsert,
	DictLookup,
	DictRemove,
	DictMap,
	DictFilter,
	/// `__hash(value) -> $int` — a structural hash consistent with `__eq` (equal
	/// values hash equal), keying the `$dict` probe table. Recurses into variant/
	/// tuple/list/record payloads like `__eq`; the exact mixing is internal (it need
	/// only agree with `__eq`, so user `hash` instances don't constrain it
	/// — the hash is a pure accelerator). FNV-1a over the structural encoding.
	Hash,
	/// `__dict_empty(unit) -> $dict` — a fresh empty mutable table (an initial-size
	/// probe array + an empty `order` list). The `map`/`filter`/`remove` rebuilds
	/// and `dict.empty` all start from one.
	DictEmpty,
	/// `__dict_find(dict, key) -> $dentry|null` — probe the table for `key`, returning
	/// its entry (so the caller reads the value, or sees null = absent). Backs
	/// `__dict_lookup` (wraps in some/none) and `__dict_eq` (compares values).
	DictFind,
	/// `__dict_eq(a, b) -> i32` — dict equality: same size and every entry of `a`
	/// present in `b` with an `__eq` value (order-independent). Lets `__eq`'s dict
	/// case stay self-contained.
	DictEq,
	/// `__dict_size(dict) -> $int` — `dict.size`: the `order` list's length (the live
	/// entry count; there are no tombstones).
	DictSize,
	/// `__dict_entries(dict) -> list` — `dict.entries`: the `order` entries as
	/// `$tuple(key, value)` in insertion order. `dict.keys/values/map/filter/merge`
	/// and the to-string/wire formatters all funnel through.
	DictEntries,
	/// `__dict_update(dict, key, f) -> nothing` — `dict.update`: a single-probe
	/// read-modify-write. `f` receives `some(current)`/`none` and returns the new
	/// value, written in place (a fused `lookup`+`insert`).
	DictUpdate,
	/// `__dict_clear(dict) -> dict` — `dict.clear`: a fresh empty dict.
	DictClear,
	/// `__cnode_lookup(node, key, hash, shift) -> $dentry|null` — descend the trie
	/// from `node`, returning the matching entry or null. `hash`/`shift` are boxed
	/// ints (the full key hash, computed once at the wrapper, and the current bit
	/// offset). Recurses; null `node` = absent.
	CnodeLookup,
	/// `__cnode_insert(node, key, val, hash, shift) -> node` — persistent insert: a
	/// path-copied trie node with `key`→`val` set (a fresh single-leaf node when
	/// `node` is null). Splits a leaf collision into a sub-node via `__cnode_merge`.
	CnodeInsert,
	/// `__cnode_merge(dA, dB, shift) -> node` — build the sub-node holding two
	/// distinct-key leaves whose hashes agree up to `shift` (recursing while their
	/// chunks collide; a flat collision bucket once the hash is exhausted).
	CnodeMerge,
	/// `__cnode_remove(node, key, hash, shift) -> node` — persistent remove: a
	/// path-copied node with `key` cleared (unchanged when absent). No canonical
	/// re-compaction in the uncompressed HAMT layout — emptied slots are left null.
	CnodeRemove,
	/// `__cnode_collect(node, list) -> nothing` — append every `(key, value)` tuple
	/// under `node` to `list` (in-place `__list_push`), recursing into sub-nodes.
	/// Backs `__dict_entries`.
	CnodeCollect,
	/// `__cnode_tinsert(node, key, val, hash, shift, token) -> node` — the transient
	/// (in-place / copy-on-write) insert the stdlib builders use; mutates only nodes
	/// owned by the current `token`. See `helpers/dict.rs`.
	CnodeTInsert,
	/// `__cnode_count(node) -> $int` — leaf count under `node` (for the `from-entries`
	/// size, since duplicates collapse during the build).
	CnodeCount,
	/// `__dict_from_entries(list) -> $dict` — `dict.from-entries`, built transiently
	/// (one owner token, `tinsert` each pair, then `count`).
	DictFromEntries,
	/// `__dict_mint_token() -> $value` — a fresh transient owner token. Minted once
	/// at the head of a linear dict region by the reuse pass (`ir::reuse`), then
	/// threaded into every `__dict_insert_into` in that region.
	DictMintToken,
	/// `__dict_insert_into(dict, key, val, token) -> $dict` — the transient analogue
	/// of `__dict_insert`: in-place when the touched nodes are owned by `token`,
	/// copy-on-write otherwise. Emitted only where the reuse pass proved the input
	/// dict uniquely owned and dead-after, so the mutation is unobservable.
	DictInsertInto,
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
	/// `__io_result(payload) -> result` — wrap a `std/sys/io` host import's return into
	/// `ok payload` (non-null) or `err (io-last-error())` (null). Keeps the host
	/// trafficking only in primitive `$value`s (string/bytes/list/nothing), never
	/// the `result` enum's variant layout — that lives here. Backs the file/stdin
	/// builtins (`io-read-file`, `io-read`, `io-read-dir`, the writers, …).
	IoResult,
	// --- async runtime (the hand-emitted task/scope driver, `helpers/task.rs`) ---
	/// `__task_drive(root) -> value` — the single-fiber poll driver. Runs a cold
	/// `$task` to completion (a Start/Ok/Err
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
	/// `__drain_next(handle) -> $tuple(action, val)` — `s.next` on a manual scope.
	DrainNext,
	/// `__run_timers()` — fire the earliest virtual timer(s) (advances the clock).
	RunTimers,
	/// `__sched_cancel(handle, _)` — `s.cancel` (queues a deferred cancellation).
	SchedCancel,
	/// `__sched_cancel_after(handle, duration)` — `s.cancel-after` (deadline timer).
	SchedCancelAfter,
	// --- marshalling boundary (the wasm↔host scratch-memory ABI, `helpers/marshal.rs`) ---
	/// `__alloc(n) -> ptr` — the scratch bump allocator: reserve `n` bytes in the
	/// exported linear memory (growing it as needed), returning the start offset.
	/// The bump cursor (`Runtime.bump`) resets to 0 at the start of each host call's
	/// arg-encoding; payloads bump within the call (host calls are synchronous).
	MarshalAlloc,
	/// `__store_bytes(b, ptr) -> ()` — copy a GC `$bytes` array into scratch at
	/// `ptr` (the wasm→host byte-payload primitive: `print`, write-file, …).
	MarshalStore,
	/// `__load_bytes(ptr, len) -> $bytes` — copy `len` scratch bytes at `ptr` into a
	/// fresh GC `$bytes` (the host→wasm byte-payload primitive: read-file, `float_to_str`).
	MarshalLoad,
	/// `__send_bytes(b) -> len` — reset the bump cursor and copy a GC `$bytes` into
	/// scratch at offset 0, returning its length. The single-payload convenience the
	/// writer emit sites (`print`/`io.write*`/`io.fail`) and the `print`-as-value
	/// wrapper share; the writer then calls its host import with `(ptr=0, len)`.
	MarshalSend,
	/// `__read_names(ptr, len) -> $list` — split a NUL-terminated name blob (the
	/// `io.read-dir` host return) in scratch into a `$list` of `$str`. Each name ends
	/// in a NUL; an empty blob is the empty list.
	MarshalReadNames,
	/// `__entry_error(value) -> i32 len` — probe `_entry`'s return for a `result.err e`
	/// (structurally: a variant whose name's last `.`-segment is `err`, one payload),
	/// render `e` into scratch via `__tostring`, and return its length, or `-1` if not
	/// an error. Exported as `__entry_error` so the host detects a program failure
	/// without reflecting the GC value. Reuses `__tostring` + `__send_bytes`.
	EntryError,
	/// `__dom_register(closure) -> i32 token` — append a `std/web/dom` event-handler
	/// closure to the `dom_handlers` registry `$list` (lazily creating it), returning
	/// its index. `dom.on-click` calls this and hands the token to the host. Reuses
	/// `__list_push`; types as `(value) -> i32` (the `EntryError` shape).
	DomRegister,
	/// `__dom_dispatch(i32 token, externref event) -> ()` — the exported event entry:
	/// look up the handler closure at `token` in `dom_handlers` and invoke it (arity-1
	/// with the boxed event). The host calls it when a registered DOM event fires.
	DomDispatch,
	/// `__browser_run() -> ()` — the browser command pump: drain ready fibers, then arm
	/// a real `setTimeout` for the soonest parked timer (or quiesce) and return.
	BrowserRun,
	/// `__browser_resume() -> ()` (exported) — the host `setTimeout` target: advance the
	/// clock to the due deadline (`__run_timers`) and re-pump.
	BrowserResume,
	/// `__browser_entry(env) -> value` (exported `_entry` for a Browser MVU build) —
	/// init the scheduler, seed `main`'s task, pump once, return.
	BrowserEntry,
	/// `__spawn_command(task) -> value` — spawn an MVU command (`task msg`) as a
	/// root-scoped fiber; its dispatch tail re-enters `update`.
	SpawnCommand,
	/// `__spawn_sub(task) -> sid (boxed int)` — spawn a keyed MVU subscription
	/// (`std/web/app`): start `task` (a `task nothing` driving a stream) as the
	/// body of a fresh detached scope and return that scope id. Unlike
	/// `spawn-command` (root-scoped, uncancellable), a subscription gets its own
	/// scope so `cancel-sub` can tear down exactly one stream.
	SpawnSub,
	/// `__cancel_sub(sid) -> nothing` — `cancel-sub`: queue subscription scope `sid`
	/// for cancellation, run between scheduler steps (so the stream driver's
	/// shielded `release` → `channel-close` `defer` runs there). The 1-arg sibling
	/// of `__sched_cancel`.
	CancelSub,
	/// `__local_get(cell) -> value` — read a task-local cell: walk the current
	/// fiber's binding env (`fibers[current_fiber].ENV`, a cons-chain of
	/// `[cell, val, next]`) for `cell` (matched by `ref.eq`), returning its bound
	/// value or the cell's default. Async-only (it indexes the scheduler globals);
	/// a non-async `local.get` is emitted inline as a bare default read.
	LocalGet,
	/// `__local_enter(cell, val) -> old-env` — push `[cell, val]` onto the current
	/// fiber's binding env and return the previous env (for `local-exit` to restore).
	/// The synchronous half of `local.with`.
	LocalEnter,
	/// `__local_exit(old-env) -> nothing` — restore the current fiber's binding env
	/// to `old-env`. The `defer`'d teardown of `local.with`.
	LocalExit,
	/// `__rpc_stream_alloc(i32 token, i32 n) -> i32 ptr` (exported) — reserve `n`
	/// scratch bytes for the host to write the next stream event's payload into,
	/// before it calls `__rpc_stream_event`. `token` is ignored (the scratch region
	/// is shared); it's in the signature so the host's call shape is uniform.
	RpcStreamAlloc,
	/// `__rpc_stream_event(i32 token, i32 kind, i32 ptr, i32 len) -> ()` (exported) —
	/// the browser loader pushes one parsed SSE event into channel `token`: `next`
	/// (kind 0) enqueues the `len` payload bytes at `ptr`, `done` (1) / `fault` (2)
	/// set the terminal flags. Then it re-readies the channel's parked puller (if
	/// any) and pumps the scheduler (`__browser_run`).
	RpcStreamEvent,
	/// `__rpc_stream_open(req) -> value` — `rpc-stream-open` (`std/web/stream`): mint a
	/// fresh channel in the `rpc_channels` registry, marshal the request `$str` into
	/// scratch, ask the host to start the `fetch` (`rpc-stream-open` import) keyed by
	/// the new token, and return `task.return token` (the resource the stream owns).
	RpcStreamOpen,
	/// `__rpc_stream_close(token) -> value` — `rpc-stream-close`: ask the host to abort
	/// the subscription's `fetch` reader (`rpc-stream-close` import) and return
	/// `task.return ()`.
	RpcStreamClose,
	/// `__web_fetch(req) -> value` — the browser lowering of `web-fetch` (`std/web/fetch`):
	/// the single-shot degenerate case of `__rpc_stream_open`. Mint a fresh channel,
	/// marshal the request `$str` into scratch, ask the host to start the unary async
	/// `fetch` (`web-fetch-open` import) keyed by the new token, and return a `WEB_FETCH`
	/// task carrying the token. The host pushes the whole reply as one `next` event then
	/// `done`; the pump's `WEB_FETCH` arm pulls it and shapes `result string string`.
	/// Shares `build_rpc_stream_open_fn` with the stream open — only the task kind and
	/// the import differ. The sys host uses the blocking path instead (no helper).
	WebFetch,
	/// `__variant_payload(value) -> valarray` — materialize a `$variant`'s inline
	/// payload (`p0`/`p1`, or `rest` for arity ≥ 3) as a uniform array, for the
	/// generic consumers (eq/wire/to-string/…) that iterate a payload of
	/// runtime-unknown arity.
	VariantPayload,
	/// `__variant_from_array(i32 vtag, value name, valarray arr) -> value` — build a
	/// `$variant` from a payload array, splitting it into the inline slots. Used by
	/// the cold/dynamic construction sites (wire decode, host-result wrappers) that
	/// already have the payload as an array.
	VariantFromArray,
	/// `__tuple_elems(value) -> valarray` — materialize a `$tuple`'s inline elements
	/// (`e0`/`e1`/`e2`, plus `rest` for arity ≥ 4) as a uniform array, for the generic
	/// consumers (eq/wire/to-string) that iterate an arity-unknown tuple.
	TupleElems,
	/// `__tuple_from_array(valarray) -> value` — build a `$tuple` from an elements
	/// array, splitting it into the inline slots. The cold/dynamic construction sites.
	TupleFromArray,
	/// `__denominalize(value) -> value` — lift a *nominal* `$shapeN` record (tag
	/// `TAG_SHAPE`) to the uniform `$record`, so the name-scanning generic consumers
	/// can handle every record uniformly; passes any other value through unchanged.
	Denominalize,
}

impl Helper {
	/// Variant count; the discriminants are `0..COUNT`, used to index
	/// `HelperIndices`. A test in `helpers` checks `REGISTRY` stays this length
	/// and in-order.
	pub(crate) const COUNT: usize = 103;
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
	/// Wasm index of the mutable `i32` global holding the scratch bump cursor (the
	/// next free offset in the exported linear memory). Always allocated (`module.rs`
	/// emits the memory + this global unconditionally), so this is a real index even
	/// in a marshalling-free program. The marshalling helpers (`helpers/marshal.rs`)
	/// read/advance it; each host-call arg-encoding resets it to 0 first.
	pub(crate) bump: u32,
	/// `result` `ok`/`err` tags + display names (for `task.attempt` and root
	/// failure) and the `__defers` field name the driver scans for.
	pub(crate) tasklits: TaskLits,
	/// `result` `ok`/`err` tags + display names `__io_result` wraps an io host
	/// return in. Populated when any `std/sys/io` result builtin is reachable.
	pub(crate) ioreslits: IoResultLits,
	/// Host import `io-last-error() -> $str` — the message `__io_result` attaches to
	/// `err` on a failed io call. Present whenever `IoResult` is emitted.
	pub(crate) io_last_error: Option<u32>,
	/// Host import `io-copyout(dst) -> ()` — drains the read-overflow stash. `Some` when an
	/// offload-fs read is reachable (the only offload op whose payload can exceed the first
	/// buffer); the pump's `FILE_READ` settle calls it.
	pub(crate) io_copyout: Option<u32>,
	/// The `std/sys/net` host import indices (the seven socket ops). `Some` exactly when
	/// the program reaches a net builtin; the pump's NET arms and the emit-side sync
	/// shaping read it.
	pub(crate) net: Option<NetImports>,
	/// The shared offload-reactor controls (`io-poll`/`io-unwatch`). `Some` when the
	/// program reaches any async-I/O builtin (`net` *or* offload); the scheduler block
	/// step + reap read it.
	pub(crate) io: Option<IoImports>,
	/// The `BlockingPool` offload-client host imports (`offload-sleep`, and the async-fs
	/// ops next). `Some` exactly when an offload builtin is reached; the pump's offload
	/// arms read it.
	pub(crate) offload: Option<OffloadImports>,
	/// Wasm index of the mutable `(ref null $list)` global holding the `std/web/dom`
	/// event-handler registry — a `$list` of handler closures, indexed by the i32
	/// token `dom.add-listener` hands the host. `Some` exactly when a `dom-add-listener`
	/// is reachable (the program registers an event handler); the exported
	/// `__dom_dispatch` reads it to find the closure for a fired event.
	pub(crate) dom_handlers: Option<u32>,
	/// Host import index of `dom-set-timeout` — the browser command pump (`__browser_run`)
	/// calls it to arm a real timer. `Some` on a Browser MVU build (when `BrowserRun` is
	/// reachable).
	pub(crate) dom_set_timeout: Option<u32>,
	/// Wasm index of the mutable `(ref null $list)` global holding the host-fed RPC
	/// stream channel registry — a `$list` of channel records (see `rpc_chan`),
	/// indexed by the token `rpc-stream-open` hands the host. `Some` exactly when the
	/// program reaches an `rpc-stream-*` builtin (a browser RPC subscription); the
	/// pump's `RPC_NEXT` arm and the exported `__rpc_stream_event` both read it.
	pub(crate) rpc_channels: Option<u32>,
	/// Host import index of `rpc-stream-open(ptr, len, token) -> ()` — start the
	/// browser `fetch` for a subscription. `Some` with `rpc_channels`.
	pub(crate) rpc_stream_open: Option<u32>,
	/// Host import index of `rpc-stream-close(token) -> ()` — abort the browser
	/// `fetch` reader for a subscription. `Some` with `rpc_channels`.
	pub(crate) rpc_stream_close: Option<u32>,
	/// Host import index of `web-fetch-open(ptr, len, token) -> ()` — start the unary
	/// browser `fetch` (`std/web/fetch`). `Some` when `web-fetch-open` is reachable;
	/// `__web_fetch_open` calls it. Shares the channel registry (`rpc_channels`) with the
	/// streaming path.
	pub(crate) web_fetch_open: Option<u32>,
}

impl Runtime {
	/// The wasm index of helper `h`, if the program emitted it.
	pub(crate) fn idx(&self, h: Helper) -> Option<u32> {
		self.helpers.get(h)
	}
}

/// The `std/sys/net` host import indices. The synchronous ops (`listen`/`close`/
/// `local_addr`) are marshalled at `emit`'s `host_call`; the suspending ops
/// (`accept`/`read`/`write`, plus `connect` — offloaded to a pool worker) are called from
/// the hand-emitted scheduler (`helpers/task.rs`). The reactor controls (`poll`/`unwatch`)
/// are no longer here — they
/// moved to the shared `IoImports`, since offload-only programs (fs, db) drive the same
/// block step without any socket op.
#[derive(Clone, Copy, Default)]
pub(crate) struct NetImports {
	pub(crate) listen: u32,
	pub(crate) close: u32,
	pub(crate) local_addr: u32,
	pub(crate) connect: u32,
	pub(crate) accept: u32,
	pub(crate) read: u32,
	pub(crate) write: u32,
}

/// The shared offload-reactor controls (host/src/offload.rs), driven by the hand-emitted
/// scheduler regardless of which async-I/O client is in play: `io-poll` is the block
/// step (block until a socket is ready or a worker completion lands), `io-unwatch` drops
/// a parked wait on reap. `Some` when the program reaches *any* async-I/O builtin
/// (`uses_net || uses_offload`) — both feed the one poll step.
#[derive(Clone, Copy, Default)]
pub(crate) struct IoImports {
	pub(crate) poll: u32,
	pub(crate) unwatch: u32,
}

/// The `BlockingPool` offload-client host import indices (host/src/offload.rs): non-pollable
/// blocking ops run on a worker thread, parked on `wait::IO` like the suspending net ops
/// and woken through the same `io-poll`. `sleep` is the v0 proving op (`offload-sleep`);
/// the async-fs ops land here next (v1). `Some` exactly when an offload builtin is reached.
#[derive(Clone, Copy, Default)]
pub(crate) struct OffloadImports {
	pub(crate) sleep: u32,
	/// `fs-op` — the generic `std/sys/fs` op (op-code in the payload selects which).
	pub(crate) op: u32,
	/// `db-op` — the generic `std/sys/db` op (open/execute/close by op-code), run on the
	/// pinned SQLite worker rather than the general pool.
	pub(crate) db: u32,
}

/// The marshalling helper/global indices the suspending net ops (`accept`/`read`/
/// `write`) need in the pump: encode the write payload + read buffer into scratch
/// (`alloc`/`store`), copy the read result out (`load`), shape the result (`io_result`,
/// the `ok`/`err` wrapper net reuses from `std/sys/io`), and the bump cursor. `Some`
/// exactly when the program reaches a net builtin (the same condition as `NetImports`).
#[derive(Clone, Copy)]
pub(crate) struct NetMarshal {
	pub(crate) alloc: u32,
	pub(crate) store: u32,
	pub(crate) load: u32,
	pub(crate) io_result: u32,
	pub(crate) bump: u32,
	/// `io-copyout` host import — drains the host's overflow stash when a read's bytes
	/// didn't fit the first `(dst, cap)` buffer. `Some` only for offload-fs reads (whose
	/// payload size is unknown); net ops read with a caller-bounded cap and never overflow.
	pub(crate) copyout: Option<u32>,
}

/// Whether `tag` is one of the seven `std/sys/net` socket builtins (the suspending
/// `accept`/`read`/`write` plus the synchronous `listen`/`close`/`local-addr`/
/// `connect`). Drives net-import registration (`module.rs`).
pub(crate) fn is_net_builtin(tag: &str) -> bool {
	is_net_sync(tag) || matches!(tag, "net-accept" | "net-read" | "net-write" | "net-connect")
}

/// Whether `tag` is a `BlockingPool` offload builtin (host/src/offload.rs) — a suspending `$task`
/// kind whose blocking call the scheduler offloads to a host worker thread. Drives
/// offload-import registration (`module.rs`), like `is_net_builtin` does for net.
pub(crate) fn is_offload_builtin(tag: &str) -> bool {
	matches!(tag, "offload-sleep" | "fs-op" | "db-op")
}

/// Whether `tag` is a *synchronous* `std/sys/net` op — a host call shaped into a
/// `result` (or a Pure `$task`, for `connect`) at the `emit` call site, rather
/// than a suspending `$task` the scheduler drives.
pub(crate) fn is_net_sync(tag: &str) -> bool {
	matches!(tag, "net-listen" | "net-close" | "net-local-addr")
}

/// One helper's wasm function type, resolved against the interner at emission.
/// Mirrors the `FuncTypes::for_*` constructors.
#[derive(Clone, Copy)]
pub(crate) enum Ty {
	Eq,
	/// `__variant_payload(value) -> valarray`.
	VariantPayload,
	/// `__variant_from_array(i32, value, valarray) -> value`.
	VariantFromArray,
	/// `__tuple_elems(value) -> valarray`.
	TupleElems,
	/// `__tuple_from_array(valarray) -> value`.
	TupleFromArray,
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
	MarshalAlloc,
	MarshalStore,
	MarshalLoad,
	MarshalSend,
	MarshalReadNames,
	EntryError,
	/// The exported `__dom_dispatch(i32, externref) -> ()` entry type.
	DomDispatch,
	/// A nullary thunk `() -> ()` (`__browser_run` / `__browser_resume`).
	Thunk,
	/// The exported `__rpc_stream_alloc(i32, i32) -> i32` type (shares the io-read
	/// `(i32, i32) -> i32` shape).
	RpcStreamAlloc,
	/// The exported `__rpc_stream_event(i32, i32, i32, i32) -> ()` type (shares the
	/// `dom-dev-store-set` four-i32-to-void shape).
	RpcStreamEvent,
}

impl Ty {
	pub(crate) fn resolve(self, ft: &mut FuncTypes) -> u32 {
		match self {
			Ty::Eq => ft.for_eq(),
			Ty::VariantPayload => ft.for_variant_payload(),
			Ty::VariantFromArray => ft.for_variant_from_array(),
			Ty::TupleElems => ft.for_tuple_elems(),
			Ty::TupleFromArray => ft.for_tuple_from_array(),
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
			Ty::MarshalAlloc => ft.for_marshal_alloc(),
			Ty::MarshalStore => ft.for_marshal_store(),
			Ty::MarshalLoad => ft.for_marshal_load(),
			Ty::MarshalSend => ft.for_marshal_send(),
			Ty::MarshalReadNames => ft.for_marshal_read_names(),
			Ty::EntryError => ft.for_entry_error(),
			Ty::DomDispatch => ft.for_dom_dispatch(),
			Ty::Thunk => ft.for_thunk(),
			Ty::RpcStreamAlloc => ft.for_io2(),
			Ty::RpcStreamEvent => ft.for_dom_dev_store_set(),
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
	/// The interned string pool — the `__denominalize` lift builds field-name
	/// `$str` constants from it. Every record field name is interned during the
	/// string scan, so each shape's names are present.
	pub(crate) strpool: &'a crate::scan::StrPool,
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
	/// The `io-last-error` host import index (present whenever `IoResult` is).
	pub(crate) fn io_last_error(&self) -> u32 {
		self
			.rt
			.io_last_error
			.expect("__io_result needs the io-last-error host import")
	}
}

/// FNV-1a 64-bit offset basis / prime — the standard constants, so the wasm
/// fingerprint matches the `wire` format's byte-for-byte. `OFFSET` seeds the hash
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

/// The `result` `ok`/`err` variant tags + interned display-name `(off, len)`
/// strings `__io_result` wraps a `std/sys/io` host return in: `ok payload` (non-null
/// host return) or `err (io-last-error())` (null).
#[derive(Clone, Copy, Default)]
pub(crate) struct IoResultLits {
	pub(crate) ok_tag: u32,
	pub(crate) err_tag: u32,
	pub(crate) ok_name: (u32, u32),
	pub(crate) err_name: (u32, u32),
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
/// rest is the cooperative scheduler state: the fiber and
/// scope tables, the ready deque, the virtual timer list, and the deferred-cancel
/// queue, plus the pump's outcome/park output channel. Allocated only when async.
#[derive(Clone, Copy, Default)]
pub(crate) struct TaskGlobals {
	pub(crate) act: u32, // mut ref null $valarray — current fiber's activation stack
	pub(crate) actlen: u32, // mut i32 — activation count
	pub(crate) fibers: u32, // mut ref null $value — $list of fiber field-arrays (by fid)
	pub(crate) scopes: u32, // mut ref null $value — $list of scope field-arrays (by sid)
	pub(crate) ready: u32, // mut ref null $value — $list of ready entries (fid, focus_kind, val)
	pub(crate) rhead: u32, // mut i32 — ready deque head cursor (pop_front)
	pub(crate) timers: u32, // mut ref null $value — $list of timer entries (at, kind, arg)
	pub(crate) pending: u32, // mut ref null $value — $list of scope ids to cancel between steps
	pub(crate) now: u32, // mut i64 — virtual clock (nanoseconds)
	pub(crate) root_kind: u32, // mut i32 — root outcome kind (0 = not done yet)
	pub(crate) root_val: u32, // mut ref null $value — root outcome value
	pub(crate) out_kind: u32, // mut i32 — pump output: 1 done / 2 park
	pub(crate) out_okerr: u32, // mut i32 — on done: outcome kind (ok/err); on park: wait kind
	pub(crate) out_val: u32, // mut ref null $value — on done: outcome value
	pub(crate) out_arg: u32, // mut i32 — on park: wait arg (fid/sid), or sleep nanos low bits unused
	pub(crate) out_arg64: u32, // mut i64 — on park sleep: nanos
	pub(crate) current_fiber: u32, // mut i32 — fid of the fiber the pump is currently running (or reaping). The task-local builtins (`local-get`/`-enter`/`-exit`) index `fibers[current_fiber].ENV` through it.
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
	/// The failure message a browser RPC stream's `fault` event surfaces as (the
	/// `wait::RPC` pump arm fails with this `$str`). v1 carries no host detail, like
	/// the native `http.fetch-stream` fault.
	pub(crate) stream_fault_msg: (u32, u32),
	/// The `err` message the browser `WEB_FETCH` pump arm produces when the host faults
	/// the unary fetch (network error) or ends it with no reply. It's a normal
	/// `result.err` value (not a task failure); v1 carries no host detail, like
	/// `stream_fault_msg`.
	pub(crate) web_fetch_fail_msg: (u32, u32),
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

/// Whether `tag` is a byte-payload writer host import: it takes one Pluma arg, which
/// wasm renders to bytes in scratch and passes as `(ptr, len)` to a `(i32,i32) -> ()`
/// import (the marshalling ABI). `print`/`io.write*` render via `__tostring`; the
/// `*-bytes` raw writers take the value's `$bytes` backing directly; `io.fail` renders
/// its message then traps. All return nothing.
pub(crate) fn is_byte_writer(tag: &str) -> bool {
	matches!(
		tag,
		"print"
			| "io-print"
			| "io-print-err"
			| "io-write"
			| "io-write-err"
			| "io-write-bytes"
			| "io-write-err-bytes"
			| "io-fail"
			// `std/web/sandbox` share-link: takes one string (the encoded snippet), writes
			// it into the URL fragment + clipboard, returns nothing — the byte-writer shape.
			| "share-link"
	)
}

/// Whether a byte-writer sends the value's raw `$bytes` backing (no `__tostring`
/// Display formatting) — the `io.write-bytes` pair, which write a `bytes` value.
pub(crate) fn is_raw_writer(tag: &str) -> bool {
	matches!(tag, "io-write-bytes" | "io-write-err-bytes")
}

/// A host primitive's calling shape: how many boxed args it takes, and whether it
/// returns a boxed value (vs. nothing — in which case the caller materializes the
/// Pluma `nothing` result).
pub(crate) struct HostSig {
	pub(crate) arity: usize,
	pub(crate) returns_value: bool,
}

/// The host signature for a builtin tag, or `None` if this backend doesn't yet
/// import it.
pub(crate) fn host_sig(tag: &str) -> Option<HostSig> {
	match tag {
		// stdout/stderr writers + the program-controlled abort. All take one
		// boxed arg and return nothing (`io.fail` diverges — the host traps).
		"print" | "io-print" | "io-print-err" | "io-write" | "io-write-err" | "io-write-bytes"
		| "io-write-err-bytes" | "io-fail"
		// `share-link` (std/web/sandbox) rides the same one-string-in, nothing-out shape.
		| "share-link" => Some(HostSig {
			arity: 1,
			returns_value: false,
		}),
		// `std/sys/io` reads/fs (server platform). These are marshalled at the `emit`
		// call site (`emit_io`) — args/results cross as scratch byte payloads + an i32
		// status/len, which `__io_result` wraps into `ok`/`err` (`is_io_result`). The
		// `arity` here is the logical Pluma signature (`io_kind` + `module.rs` pick the
		// actual wasm `Io2`/`Io4` import type); `host_sig` is consulted only for the
		// "is this a host builtin?" classification.
		"io-read" | "io-read-all" | "io-read-all-bytes" | "io-read-file" | "io-read-file-bytes"
		| "io-delete-file" | "io-make-dir" | "io-read-dir" | "io-cwd"
		// `io.args` rides the same marshalled-read path (`(dst,cap) -> len`, a blob in
		// scratch) but returns a bare `list string`, not a `result` (`IoKind::Args`).
		| "io-args"
		// `io.env` reads `(name,nlen,dst,cap) -> len` and shapes `len` (`-1` = unset)
		// into an `option string` (`IoKind::EnvVar`).
		| "io-env" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		// The playground primitives: compile a Pluma source string to a wasm module
		// (`compile.to-wasm-hex`, server) and run a compiled module in the browser
		// (`sandbox.run-hex`, web). Both take one string and return `result string
		// string`, so they ride the same marshalled read path as `io-read-file`
		// (`IoKind::ReadFileStr`) — see `io_kind`/`is_io_result`.
		"compile-wasm-hex" | "sandbox-run-hex" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		// `io.exit code` diverges: `(i32 code) -> ()`, the host exits the process. Not
		// `is_io_host`/`io_kind` — emitted by `emit_exit`, typed `(i32)->()` in `module`.
		"io-exit" => Some(HostSig {
			arity: 1,
			returns_value: false,
		}),
		"io-write-file" | "io-write-file-bytes" | "io-append-file" | "io-append-file-bytes" => {
			Some(HostSig {
				arity: 2,
				returns_value: true,
			})
		}
		// These return a bare `bool` (no `__io_result` wrapping).
		"io-file-exists" | "io-is-dir" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		// The error channel `__io_result` reads on a failed io/net call (errno-style:
		// the host sets `last_error` on every failing call). Marshalled `(dst,cap)->len`.
		"io-last-error" => Some(HostSig {
			arity: 0,
			returns_value: true,
		}),
		// `std/random` / `std/uuid` (Entropy). The scalars cross natively (i64 as a
		// JS BigInt, f64 direct); `random-bytes`/`uuid-v4`/`uuid-v7` write a payload to
		// scratch (`is_rng_host` → `emit_rng`); `uuid-parse` rides the io read path
		// (`ReadFileStr`) so its `result` is shaped by `__io_result`.
		"random-int" | "random-float" => Some(HostSig {
			arity: 0,
			returns_value: true,
		}),
		"random-int-range" => Some(HostSig {
			arity: 2,
			returns_value: true,
		}),
		"random-bytes" | "uuid-v4" | "uuid-v7" | "uuid-parse" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		// `std/regex`: `(pattern-source, subject) -> bytes` (a packed i32 span buffer).
		// Has a dedicated 6-arg import type + emit path (`emit_regex`); this entry only
		// keeps `classify_host_call` from rejecting it as an unsupported builtin.
		"regex-find-all" => Some(HostSig {
			arity: 2,
			returns_value: true,
		}),
		// `std/time` clock reads (`Clock`). `time-now`/`-monotonic` cross as i64
		// BigInts (boxed `instant`/`duration` in wasm — `is_clock_host` → `emit_clock`);
		// `time-parse` rides a marshalled `(fp,fl,ip,il,dst) -> status` shape into a
		// `result instant string`. `arity` is the logical Pluma signature.
		"time-now" | "time-monotonic" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		"time-parse" => Some(HostSig {
			arity: 2,
			returns_value: true,
		}),
		// `time.sleep d` diverges only in effect: `(i64 nanos) -> ()`, the host blocks.
		"time-sleep" => Some(HostSig {
			arity: 1,
			returns_value: false,
		}),
		// `std/web/dom` (the Web target). All emitted via `emit_dom` (`is_dom_host` →
		// `dom_kind`); the `arity`/`returns_value` here drive only the "is this a host
		// builtin?" classification — node handles cross as `externref` and strings as
		// scratch, which the generic host path can't shape.
		// `dom-window` returns the page's `window` (an externref) just like `dom-body`
		// returns `<body>`; the location/history helpers pass it to their imports.
		"dom-body" | "dom-window" => Some(HostSig {
			arity: 0,
			returns_value: true,
		}),
		// The location readers (`location.pathname`/`.search`/`.hash`) take the window
		// node and probe-read a string back — the same shape as `dom-get-value`.
		"dom-create-element"
		| "dom-create-text"
		| "dom-get-value"
		| "dom-location-path"
		| "dom-location-search"
		| "dom-location-hash"
		| "event-target-value"
		| "event-target-checked" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		"event-prevent-default" => Some(HostSig {
			arity: 1,
			returns_value: false,
		}),
		// `dom-child-at`: `(externref, i32) -> externref` — index a node's children.
		"dom-child-at" => Some(HostSig {
			arity: 2,
			returns_value: true,
		}),
		// `dom-push-state`/`dom-replace-state` take the window node + a URL string and
		// return nothing — the same node-plus-one-string shape as `dom-set-text`.
		"dom-append-child"
		| "dom-set-text"
		| "dom-remove-child"
		| "dom-remove-attribute"
		| "dom-push-state"
		| "dom-replace-state" => Some(HostSig {
			arity: 2,
			returns_value: false,
		}),
		"dom-set-attribute" | "dom-replace-child" | "dom-insert-before" | "dom-add-listener"
		// the property setters: node + name + (string | bool). Same arity/shape as
		// `dom-set-attribute` (the bool rides as the i32 third arg). `dom-set-style-property`
		// is node + property name + value, the same `SetAttr` shape.
		| "dom-set-string-property" | "dom-set-style-property" | "dom-set-bool-property" => {
			Some(HostSig {
				arity: 3,
				returns_value: false,
			})
		}
		// dev-only HMR store (`pluma dev`): set takes (key, value) strings; get takes a
		// key string and returns the stored value string.
		"dom-dev-store-set" => Some(HostSig {
			arity: 2,
			returns_value: false,
		}),
		// `dom-dev-store-get` and `dom-element-text` both take one key/id string and
		// probe a value string back — the same shape.
		"dom-dev-store-get" | "dom-element-text" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		// `std/web/fetch` (the Web target HTTP transport): one request string in, the
		// reply produced back. The sys host lowers it like an io read (the blocking
		// `emit_web_fetch` path, classified below); the browser routes it to the
		// `WebFetch` helper instead (intercepted in `module.rs`). `web-fetch-open` is the
		// browser's async-`fetch` import. These entries only drive the "is this a host
		// builtin?" classification.
		"web-fetch" | "web-fetch-open" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		// `std/web/stream` browser RPC subscription transport. `rpc-stream-open` starts
		// the `fetch` (request string in, a channel token back, wrapped in a task);
		// `rpc-stream-close` aborts it. Both are shaped at their emit sites
		// (`emit_rpc_stream_open`/`_close`); these entries only drive the "is this a host
		// builtin?" classification.
		"rpc-stream-open" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		"rpc-stream-close" => Some(HostSig {
			arity: 1,
			returns_value: true,
		}),
		_ => None,
	}
}

/// How a `std/time` clock host import shapes its result (`emit_clock`). The pure
/// `time` conversions (`time-duration-as-nanos` etc.) are *not* here — those are inline
/// `retag_int_box` builtins with no host import.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClockKind {
	/// `time-now`: `() -> i64`; box `{TAG_INSTANT, i64}`.
	NowInstant,
	/// `time-monotonic`: `() -> i64`; box `{TAG_DURATION, i64}`.
	MonotonicDuration,
	/// `time-sleep`: `(i64) -> ()`; unbox the `duration` arg, call, then `nothing`.
	Sleep,
	/// `time-parse`: `(fp,fl,ip,il,dst) -> status`; an i64 written to scratch on ok,
	/// shaped into `result instant string` via `__io_result`.
	Parse,
}

/// Classify a `std/time` clock host builtin (the ones needing a host import). `None`
/// for the inline conversions and non-time tags.
pub(crate) fn clock_kind(tag: &str) -> Option<ClockKind> {
	Some(match tag {
		"time-now" => ClockKind::NowInstant,
		"time-monotonic" => ClockKind::MonotonicDuration,
		"time-sleep" => ClockKind::Sleep,
		"time-parse" => ClockKind::Parse,
		_ => return None,
	})
}

/// Whether `tag` is an `emit_clock`-handled `std/time` clock host import.
pub(crate) fn is_clock_host(tag: &str) -> bool {
	clock_kind(tag).is_some()
}

/// How a `std/random`/`std/uuid` host import (other than `uuid-parse`, which rides
/// the io read path) shapes its result. The scalars box directly; the byte/string
/// ones write a payload to scratch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RngKind {
	/// `random-int`: `() -> i64`; box `$int`.
	ScalarI64,
	/// `random-float`: `() -> f64`; box `$float`.
	ScalarF64,
	/// `random-int-range`: `(i64, i64) -> i64`; box `$int`. Validated in Pluma.
	RangeI64,
	/// `random-bytes`: `(i32 n, dst, cap) -> len`; build `$bytes`. Validated in Pluma.
	BytesN,
	/// `uuid-v4`/`uuid-v7`: `(dst, cap) -> len`; build `$str` (never fails).
	UuidStr,
}

/// Classify a `std/random`/`std/uuid` builtin emitted via `emit_rng` (everything
/// but `uuid-parse`, which goes through `emit_io` as a `ReadFileStr`). `None` otherwise.
pub(crate) fn rng_kind(tag: &str) -> Option<RngKind> {
	Some(match tag {
		"random-int" => RngKind::ScalarI64,
		"random-float" => RngKind::ScalarF64,
		"random-int-range" => RngKind::RangeI64,
		"random-bytes" => RngKind::BytesN,
		"uuid-v4" | "uuid-v7" => RngKind::UuidStr,
		_ => return None,
	})
}

/// Whether `tag` is an `emit_rng`-handled entropy/uuid builtin.
pub(crate) fn is_rng_host(tag: &str) -> bool {
	rng_kind(tag).is_some()
}

/// How a `std/web/dom` host import (`the Web target`) crosses the boundary and is
/// shaped at the `emit_dom` call site. Node handles ride as `externref` (unboxed
/// from / boxed into a `$extern` wrapper); strings ride as scratch `(ptr, len)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DomKind {
	/// `dom-body`: `() -> externref`; box the returned node into `$extern`.
	Body,
	/// `dom-create-element`/`dom-create-text`: `(ptr, len) -> externref`; one scratch
	/// string in, box the returned node.
	Make,
	/// `dom-append-child`: `(externref, externref) -> ()`; unbox two node args.
	Append,
	/// `dom-set-attribute`: `(externref, np, nl, vp, vl) -> ()`; node + two strings.
	SetAttr,
	/// `dom-set-text`: `(externref, ptr, len) -> ()`; node + one string.
	SetText,
	/// `dom-get-value` / `event-target-value`: `(externref, dst, cap) -> len`; node/event
	/// in, probe-read a `$str` (the input's `.value`, or the event target's).
	GetValue,
	/// `dom-add-listener`: `(externref, np, nl, token) -> ()`; node + an event-name string
	/// + a registry token. The handler closure (the third Pluma arg) is pushed into the
	/// handler registry and replaced by its i32 index — the host wires
	/// `addEventListener(name, e => __dom_dispatch(token, e))`.
	Listen,
	/// `dom-remove-child`: `(externref, externref) -> ()`; unbox two node args (like `Append`).
	Append2,
	/// `dom-replace-child` / `dom-insert-before`: `(externref, externref, externref) -> ()`;
	/// unbox three node args.
	Extern3,
	/// `dom-remove-attribute`: `(externref, ptr, len) -> ()`; node + one string (like `SetText`).
	NodeStr,
	/// `event-prevent-default`: `(externref) -> ()`; unbox one node/event arg.
	Extern1,
	/// `dom-child-at`: `(externref, i32) -> externref`; unbox the node + index, box the
	/// returned child node. Used by `render.hydrate` to walk a server-rendered tree.
	ChildAt,
	/// `dom-set-string-property`: `(externref, np, nl, vp, vl) -> ()`; node + name + value
	/// string -- assigns a DOM *property* (`node[name] = value`), not an attribute. Same
	/// wasm shape and emit as `SetAttr`; the host does a property write, not `setAttribute`.
	SetProp,
	/// `dom-set-bool-property`: `(externref, np, nl, i32) -> ()`; node + name string + the
	/// unboxed `bool`, assigning `node[name] = !!flag`. Bools cross as i32, never a string
	/// (`node.disabled = "false"` would be truthy). Same wasm shape as `Listen`.
	SetBoolProp,
	/// `dom-dev-store-set`: `(kp, kl, vp, vl) -> ()`; two scratch strings, no node. The
	/// dev-only `localStorage` write `pluma dev`'s HMR uses to persist the model.
	DevStoreSet,
	/// `dom-dev-store-get`: `(kp, kl, dst, cap) -> len`; a scratch-string key in,
	/// probe-read the stored value into scratch (the `GetValue` shape minus the node).
	DevStoreGet,
}

/// Classify a `std/web/dom` host builtin emitted via `emit_dom`. `None` for non-dom tags.
pub(crate) fn dom_kind(tag: &str) -> Option<DomKind> {
	Some(match tag {
		"dom-body" | "dom-window" => DomKind::Body,
		"dom-create-element" | "dom-create-text" => DomKind::Make,
		"dom-append-child" => DomKind::Append,
		"dom-set-attribute" => DomKind::SetAttr,
		// `history.pushState`/`replaceState`: window node + a URL string, no result —
		// the same emit shape as `dom-set-text` (the host body differs).
		"dom-set-text" | "dom-push-state" | "dom-replace-state" => DomKind::SetText,
		// `location.pathname`/`.search`/`.hash`: window node in, string probed back out
		// — the `dom-get-value` shape (the host reads `location.*` instead of `.value`).
		"dom-get-value"
		| "dom-location-path"
		| "dom-location-search"
		| "dom-location-hash"
		| "event-target-value"
		| "event-target-checked" => DomKind::GetValue,
		"dom-add-listener" => DomKind::Listen,
		"dom-remove-child" => DomKind::Append2,
		"dom-replace-child" | "dom-insert-before" => DomKind::Extern3,
		"dom-remove-attribute" => DomKind::NodeStr,
		"event-prevent-default" => DomKind::Extern1,
		"dom-child-at" => DomKind::ChildAt,
		"dom-set-string-property" => DomKind::SetProp,
		// `node.style.setProperty(name, value)` -- same wasm shape as `SetProp`, its own
		// import (the host body differs).
		"dom-set-style-property" => DomKind::SetProp,
		"dom-set-bool-property" => DomKind::SetBoolProp,
		"dom-dev-store-set" => DomKind::DevStoreSet,
		// `dom-element-text` reads an element's text by id — a key-string-in,
		// value-string-out probe, exactly the `dev-store-get` shape.
		"dom-dev-store-get" | "dom-element-text" => DomKind::DevStoreGet,
		_ => return None,
	})
}

/// Whether `tag` is an `emit_dom`-handled `std/web/dom` host import.
pub(crate) fn is_dom_host(tag: &str) -> bool {
	dom_kind(tag).is_some()
}

/// Whether `tag` is a `std/sys/io` builtin emitted through the marshalling host path
/// (the file/stdin ops + the `bool` queries + `io.args`) — all of which traffic byte
/// payloads through scratch memory. A superset of `is_io_result`; the extras
/// (`io-file-exists`/`io-is-dir` return a bare `bool`, `io-args` a bare `list`) skip
/// `__io_result`.
pub(crate) fn is_io_host(tag: &str) -> bool {
	is_io_result(tag) || matches!(tag, "io-file-exists" | "io-is-dir" | "io-args" | "io-env")
}

/// How a marshalled `std/sys/io` op crosses the boundary: the wasm host-import shape
/// (`Io2` = `(i32,i32) -> i32`, `Io4` = `(i32,i32,i32,i32) -> i32`) plus how the emit
/// site shapes the `i32` result back into a `$value`. `Read*` ops length-probe a
/// `dst`; the writers return a `status`; the queries return a `bool`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoKind {
	/// `(dst, cap) -> len`; build a `$str` (`io-read`/`io-read-all`/`io-last-error`).
	ReadStr,
	/// `(dst, cap) -> len`; build a `$bytes` (`io-read-all-bytes`).
	ReadBytes,
	/// `(path, plen, dst, cap) -> len`; build a `$str` (`io-read-file`).
	ReadFileStr,
	/// `(path, plen, dst, cap) -> len`; build a `$bytes` (`io-read-file-bytes`).
	ReadFileBytes,
	/// `(path, plen, dst, cap) -> len`; split into a `$list` of `$str` (`io-read-dir`).
	ReadDir,
	/// `(dst, cap) -> len`; split a NUL-blob into a bare `$list` of `$str` — `io.args`.
	/// Like `ReadDir` minus the path arg and the `__io_result` wrap (argv never fails).
	Args,
	/// `(name, nlen, dst, cap) -> len`; shape `len` (`-1` = unset) into an `option
	/// string` — `io.env`. Like a path read, but wraps `some`/`none`, not `ok`/`err`.
	EnvVar,
	/// `(path, plen, data, dlen) -> status`; `nothing` payload (`io-write-file*`/`-append*`).
	WriteFile,
	/// `(path, plen) -> status`; `nothing` payload (`io-delete-file`/`io-make-dir`).
	PathStatus,
	/// `(path, plen) -> bool` (`io-file-exists`/`io-is-dir`).
	PathBool,
	/// `(op, path, plen, data, dlen, dst, cap) -> len`; the synchronous `std/sys/fs` twin
	/// (`fs-op-sync`). One op-code dispatch like the async `fs-op`, returning the op's bytes
	/// through `(dst, cap)` (overflow → `io-copyout`), wrapped `ok`/`err` by `__io_result`;
	/// the Pluma `-sync` wrapper interprets the bytes per op.
	FsOpSync,
}

/// Classify a marshalled `std/sys/io` builtin tag (and `io-last-error`, an internal
/// read). `None` for non-io tags.
pub(crate) fn io_kind(tag: &str) -> Option<IoKind> {
	Some(match tag {
		"io-read" | "io-read-all" | "io-last-error" | "io-cwd" => IoKind::ReadStr,
		"io-read-all-bytes" => IoKind::ReadBytes,
		// `uuid-parse` isn't io, but it has the same shape — a string in, a `result
		// string` out — so it reuses the `(path, plen, dst, cap)` read marshalling.
		// `compile-wasm-hex`/`sandbox-run-hex` aren't io either, but share the shape —
		// a string in, a `result string` out (the playground compile + run primitives).
		"io-read-file" | "uuid-parse" | "compile-wasm-hex" | "sandbox-run-hex" => IoKind::ReadFileStr,
		"io-read-file-bytes" => IoKind::ReadFileBytes,
		"io-read-dir" => IoKind::ReadDir,
		"io-args" => IoKind::Args,
		"io-env" => IoKind::EnvVar,
		"io-write-file" | "io-write-file-bytes" | "io-append-file" | "io-append-file-bytes" => {
			IoKind::WriteFile
		}
		"io-delete-file" | "io-make-dir" => IoKind::PathStatus,
		"io-file-exists" | "io-is-dir" => IoKind::PathBool,
		"fs-op-sync" => IoKind::FsOpSync,
		_ => return None,
	})
}

/// Whether a marshalled io op uses the four-arg host shape (`Io4`) — the path reads
/// and the file writers; the rest are two-arg (`Io2`).
pub(crate) fn io_uses_io4(tag: &str) -> bool {
	matches!(
		io_kind(tag),
		Some(
			IoKind::ReadFileStr
				| IoKind::ReadFileBytes
				| IoKind::ReadDir
				| IoKind::WriteFile
				| IoKind::EnvVar
		)
	)
}

/// Whether `tag` is a `std/sys/io` builtin whose host return must be wrapped into a
/// `result` by `__io_result` (the file/stdin ops returning `result …`). Excludes
/// `io-file-exists`/`io-is-dir` (bare `bool`) and the stdout writers (`nothing`).
pub(crate) fn is_io_result(tag: &str) -> bool {
	matches!(
		tag,
		"io-read"
			| "io-read-all"
			| "io-read-all-bytes"
			| "io-read-file"
			| "io-read-file-bytes"
			| "io-write-file"
			| "io-write-file-bytes"
			| "io-append-file"
			| "io-append-file-bytes"
			| "io-delete-file"
			| "io-make-dir"
			| "io-read-dir"
			| "io-cwd"
			// rides the io read path; its `result string` is shaped by `__io_result`.
			| "uuid-parse"
			// the sync `std/sys/fs` op — its `result bytes` is shaped by `__io_result`.
			| "fs-op-sync"
			// the playground compile + run primitives — `result string` via `__io_result`.
			| "compile-wasm-hex"
			| "sandbox-run-hex"
	)
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
			// the in-place handler-registry overwrite: `array.set` on `dom_handlers`.
			| "dom-set-handler"
			// mutable cells: a `$ref` struct with a mutable boxed-value field.
			// `ref-update` reads, applies a closure, writes back (closure call inline).
			| "ref-new"
			| "ref-get"
			| "ref-set"
			| "ref-update"
			// task-local cell alloc: a `$local` struct holding the default value
			// (identity by `ref.eq`). `get`/`enter`/`exit` are scheduler helpers (they
			// index the current fiber's env), routed in `emit::host_call`.
			| "local-new"
			// math primitives WasmGC does with one f64/i64 opcode (the
			// transcendentals log/exp/sin/cos still need a host import).
			| "math-sqrt"
			| "math-to-int"
			| "math-to-float"
			// bitwise ops over the i64 payload: one WasmGC integer opcode each
			// (`not` is `xor -1`).
			| "bit-and"
			| "bit-or"
			| "bit-xor"
			| "bit-not"
			| "bit-shift-left"
			| "bit-shift-right"
			| "bit-shift-right-unsigned"
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
		// std/sys/net suspending socket ops: a `$task` carrying the op's args (the
		// scheduler does the non-blocking host call + reactor park, or the offload submit
		// for connect). listen/close/local-addr are NOT here — they're synchronous calls.
		"net-accept" => task_kind::NET_ACCEPT,
		"net-read" => task_kind::NET_READ,
		"net-write" => task_kind::NET_WRITE,
		// connect is offloaded to a pool worker (blocking DNS + handshake), not a readiness
		// park; the host submits the dial and hands back the socket on collect.
		"net-connect" => task_kind::NET_CONNECT,
		// `std/web/stream`: pull the next host-fed RPC stream event (a `$task` the
		// scheduler drives — dequeue or park on `wait::RPC`).
		"rpc-stream-next" => task_kind::RPC_NEXT,
		// BlockingPool offload ops (host/src/offload.rs): a `$task` carrying the op's args; the
		// scheduler submits the blocking call to a worker thread + parks on `wait::IO`.
		"offload-sleep" => task_kind::OFFLOAD_SLEEP,
		"fs-op" => task_kind::FILE_OP,
		"db-op" => task_kind::DB_OP,
		_ => return None,
	})
}

/// Transcendental float math with no WasmGC opcode (log/log10/log2/exp/sin/cos).
/// Each is a `(f64) -> f64` host import (the libm call); the
/// `$float` box/unbox is emitted in wasm around the call.
pub(crate) fn is_f64_unary_host(tag: &str) -> bool {
	matches!(
		tag,
		"math-log" | "math-log10" | "math-log2" | "math-exp" | "math-sin" | "math-cos"
	)
}
