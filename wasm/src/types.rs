// The WasmGC type section: the `$value` rec group plus its scalar/heap subtypes,
// and arity-keyed function types.
//
// The WASM backend gives `int`→i64, `float`→f64, `bool`→i32 *locals* and
// represents every *boxed* (`Repr::Boxed`) value as a GC reference to a `$value`
// subtype. The `$value` supertype carries an `i32` discriminant tag so polymorphic
// code and tag-inspecting ops (`Match`, structural `Eq`, host-side `print`
// formatting) can read the runtime kind off any reference.
//
// Tag values are a cross-cutting contract: the emitter writes them, the host
// `print`/`debug` glue reads them to format a value. Keep `tag` in sync with the
// host's value formatter (`host/src/v8host.rs`).

use crate::util::repr_valtype;
use ir::Repr;
use wasm_encoder::{
	AbstractHeapType, CompositeInnerType, CompositeType, FieldType, HeapType, RefType, StorageType,
	StructType, SubType, TypeSection, ValType,
};

// --------------------------------------------------------------------------
// Concrete type indices. The scalar/heap subtypes occupy a fixed prefix; the
// arity-keyed function types follow, assigned by `FuncTypes`.
// --------------------------------------------------------------------------

pub const T_VALUE: u32 = 0; // struct { i32 tag }                 — the boxed supertype
pub const T_BYTES: u32 = 1; // array (mut i8)                     — UTF-8 / raw bytes backing
pub const T_INT: u32 = 2; // struct { i32 tag, i64 }
pub const T_FLOAT: u32 = 3; // struct { i32 tag, f64 }
pub const T_BOOL: u32 = 4; // struct { i32 tag, i32 }
pub const T_STR: u32 = 5; // struct { i32 tag, (ref $bytes) }
pub const T_VALARRAY: u32 = 6; // array (mut (ref null $value))   — captures / payload backing
pub const T_CLOSURE: u32 = 7; // struct { i32 tag, i32 fn_index, (ref $valarray) captures }
pub const T_VARIANT: u32 = 8; // struct { i32 tag, i32 vtag, (ref $str) name, i32 arity, p0, p1, (ref null $valarray) rest } — payload inline
pub const T_CTOR: u32 = 9; // struct { i32 tag, i32 vtag, i32 arity }  — a partial variant ctor
pub const T_METHODDICT: u32 = 10; // struct { i32 tag, (ref $valarray) methods }
pub const T_TUPLE: u32 = 11; // struct { i32 tag, (ref $valarray) elems }
pub const T_LIST: u32 = 12; // struct { i32 tag, (ref $valarray) elems }
pub const T_RECORD: u32 = 13; // struct { i32 tag, (ref $valarray) names, (ref $valarray) values }
pub const T_REF: u32 = 14; // struct { i32 tag, (mut ref null $value) cell }  — a mutable cell
pub const T_DICT: u32 = 15; // struct { i32 tag, (mut ref null $value) root, (mut i32) size }  — persistent HAMT
pub const T_TASK: u32 = 16; // struct { i32 tag, i32 kind, (ref $valarray) payload }  — a cold async `task`
pub const T_DENTRY: u32 = 17; // struct { i32 tag, (ref null $value) key, (mut ref null $value) value, i64 hash }  — a $dict entry
pub const T_EXTERN: u32 = 18; // struct { i32 tag, (ref null extern) handle }  — a host-owned resource handle
pub const T_CNODE: u32 = 19; // struct { i32 tag, i32 dataMap, i32 nodeMap, (mut ref $valarray) entries, (mut ref $valarray) children, (mut ref null $value) edit }  — a persistent dict trie node
pub const T_LOCAL: u32 = 20; // struct { i32 tag, (ref null $value) default }  — a task-local cell (identity by ref.eq)
const T_FIRST_FUNC: u32 = 21;

// --------------------------------------------------------------------------
// Runtime tags carried in the `$value` discriminant field — one per runtime
// value kind; the host formatter switches on these.
// --------------------------------------------------------------------------

pub const TAG_NOTHING: i32 = 0;
pub const TAG_BOOL: i32 = 1;
pub const TAG_INT: i32 = 2;
pub const TAG_FLOAT: i32 = 3;
pub const TAG_STR: i32 = 4;
pub const TAG_DURATION: i32 = 5;
pub const TAG_INSTANT: i32 = 6;
pub const TAG_CLOSURE: i32 = 7;
pub const TAG_VARIANT: i32 = 8;
pub const TAG_CTOR: i32 = 9;
pub const TAG_METHODDICT: i32 = 10;
pub const TAG_TUPLE: i32 = 11;
pub const TAG_LIST: i32 = 12;
pub const TAG_RECORD: i32 = 13;
/// A `bytes` value: same wasm shape as `$str` (struct { tag, ref $bytes }),
/// distinguished from a string only by this tag.
pub const TAG_BYTES: i32 = 14;
/// A `ref a` mutable cell: a `$ref` struct holding one (mutable) boxed value.
/// Compared by reference identity (`ref.eq`).
pub const TAG_REF: i32 = 15;
/// A `dict k v`: a `$dict` struct `{ tag, root, size }` — an **immutable
/// persistent** hash-array-mapped trie (see `helpers/dict.rs`). `root` is the top
/// `$cnode` (or null when empty); `size` caches the live entry count. `insert`/
/// `remove` path-copy and return a new `$dict`; keys are matched by structural
/// `__hash` + `__eq`. The `$cnode`/`$dentry` nodes never escape to user code (a
/// `$dict` is the only handle).
pub const TAG_DICT: i32 = 16;
/// A cold, re-runnable `task a`: a `$task` struct `{ tag, i32 kind, payload }`.
/// `kind` is the `TaskRepr` discriminant (see `runtime::task_kind`); `payload`
/// holds its components. The distinct tag lets the driver detect a task at the
/// program root (the boxed-task discriminant at the program root). Built and
/// consumed only by the hand-emitted async driver — never printed.
pub const TAG_TASK: i32 = 17;
/// A `scope-handle` / `manual-scope-handle`: a `$int`-shaped box (`{ tag, i64 }`)
/// carrying a scope id. The `scope-*` builtins read its id; never printed.
pub const TAG_SCOPE_HANDLE: i32 = 18;
/// A host-owned resource handle (`$extern`): a `{ tag, (ref null extern) }` wrapper
/// boxing an engine-managed `externref` (a DOM node, a `fetch` response, …) so it can
/// flow through Pluma code as an ordinary value. Compared by reference identity
/// (`ref.eq` on the wrapper, like `$ref`); Display is the opaque `<extern>`; never
/// structurally serialized (a handle must not cross the `wire`). Only the Web
/// target's DOM/fetch imports produce one.
pub const TAG_EXTERN: i32 = 19;
/// A persistent-`$dict` trie node (`$cnode`): a `$value` subtype carrying a
/// bitmap + a `$valarray` of slots (each a `$dentry` leaf or a child `$cnode`).
/// A distinct tag so the trie walk distinguishes a leaf entry from a sub-node by
/// reading field 0, without a concrete `ref.test`. Internal: never escapes to
/// user code (only a `$dict` does), never printed.
pub const TAG_CNODE: i32 = 20;
/// A `local a` task-local cell: a `$local` struct `{ tag, default }` holding the
/// cell's default value. Identity is the struct reference itself (`ref.eq`, like
/// `$ref`) — the binding environment matches frames by cell identity. Never printed.
pub const TAG_LOCAL: i32 = 21;

/// `(ref null $valarray)` — a reference to a value array (closure captures or
/// variant payload).
pub fn valarray_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_VALARRAY),
	})
}

/// `(ref null $valarray)` — the nullable form, for locals that receive a value
/// read from a nullable global (e.g. the async activation stack / wire registry,
/// which start null).
pub fn valarray_ref_null() -> ValType {
	ValType::Ref(RefType {
		nullable: true,
		heap_type: HeapType::Concrete(T_VALARRAY),
	})
}

/// `(ref null eq)` — the uniform boxed-value type used for params, results,
/// captures, and every `Boxed` local. Re-rooted from the concrete `$value` struct
/// to the abstract `eq` top so a value can be EITHER a heap `$value` subtype (as
/// before) OR an `i31ref` immediate (a small int — no allocation). Every heap
/// subtype and the typed null `ref.null $value` remain
/// valid `eqref`s by subtyping, so this single change re-types every value slot
/// (params, `$valarray` elements, value-holding fields, locals) at once; only a
/// bare `struct.get $value 0` tag-read needs an explicit `ref.cast $value` first
/// (routed through `value_tag`).
pub fn value_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: true,
		heap_type: HeapType::Abstract {
			shared: false,
			ty: AbstractHeapType::Eq,
		},
	})
}

/// `(ref $variant)` — a non-null reference to a variant struct.
pub fn variant_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_VARIANT),
	})
}

/// `(ref $ref)` — a non-null reference to a mutable cell struct.
pub fn ref_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_REF),
	})
}

/// `(ref $dentry)` — a non-null reference to a `$dict` entry struct. Used for a
/// local that holds a `ref.cast`-to-`$dentry` (so a later `struct.get` reads its
/// key/value/hash; a plain `$value` local would lose the subtype).
pub fn dentry_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_DENTRY),
	})
}

/// `(ref $cnode)` — a non-null reference to a persistent-dict trie node. Used for
/// locals holding a `ref.cast`-to-`$cnode` so a later `struct.get` reads its
/// bitmap/slots.
#[allow(dead_code)] // referenced by the persistent-dict trie walk
pub fn cnode_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_CNODE),
	})
}

/// `(ref $bytes)` — a non-null reference to the byte-array backing of a string.
pub fn bytes_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: false,
		heap_type: HeapType::Concrete(T_BYTES),
	})
}

/// `(ref null extern)` — an engine-managed host resource reference. Not an `eqref`,
/// so it can't sit in a value slot directly; it rides inside a `$extern` wrapper
/// struct (`T_EXTERN`) whose reference *is* an `eqref`. Used only as that wrapper's
/// field type today (only the Web target's imports traffic one).
pub fn extern_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: true,
		heap_type: HeapType::Abstract {
			shared: false,
			ty: AbstractHeapType::Extern,
		},
	})
}

/// `anyref` — the abstract top of the GC reference hierarchy. Host imports take
/// their boxed args as `anyref` (the wasm caller passes a `(ref null $value)`,
/// a valid subtype) so the host glue need not name the module's concrete types.
pub fn any_ref() -> ValType {
	ValType::Ref(RefType {
		nullable: true,
		heap_type: HeapType::Abstract {
			shared: false,
			ty: AbstractHeapType::Any,
		},
	})
}

fn val_field(t: ValType, mutable: bool) -> FieldType {
	FieldType {
		element_type: StorageType::Val(t),
		mutable,
	}
}

fn struct_subtype(super_idx: Option<u32>, fields: Vec<FieldType>, is_final: bool) -> SubType {
	SubType {
		is_final,
		supertype_idx: super_idx,
		composite_type: CompositeType {
			inner: CompositeInnerType::Struct(StructType {
				fields: fields.into_boxed_slice(),
			}),
			shared: false,
			descriptor: None,
			describes: None,
		},
	}
}

/// Arity-keyed function-type interner. In the uniform-boxed contract every
/// function takes `n` boxed params and returns one boxed value, so a function's
/// wasm type is fully determined by its arity. (Monomorphization will later vary
/// this; that's a follow-on.)
/// One interned function type: a Pluma function (boxed params + boxed result) or
/// a host import (`anyref` params, optional boxed result).
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
enum FuncKind {
	Pluma(usize),
	Host(usize, bool),
	/// The structural-equality runtime helper: `(value, value) -> i32`.
	Eq,
	/// `__variant_payload(value) -> valarray` — materialize a `$variant`'s inline
	/// payload as a uniform array for generic consumers.
	VariantPayload,
	/// `__variant_from_array(i32 vtag, value name, valarray arr) -> value` — build a
	/// `$variant` from a payload array (splitting it into the inline slots).
	VariantFromArray,
	/// A runtime helper taking `n` boxed args and returning a boxed value.
	Helper(usize),
	/// The array-concat helper: `(valarray, valarray) -> valarray`.
	ArrConcat,
	/// The bytes-concat helper: `(bytes, bytes) -> bytes`.
	BytesConcat,
	/// The float-format host import: `(f64, i32 ptr, i32 cap) -> i32 len`. The host
	/// formats the float and writes its UTF-8 bytes into scratch at `ptr` (≤ `cap`),
	/// returning the length; the box/unbox to `$float` happens in wasm.
	FloatToStr,
	/// A byte-payload writer host import: `(i32 ptr, i32 len) -> ()` — `print` /
	/// `io.write*` / `io.fail`. wasm pre-renders the bytes into scratch (via
	/// `__tostring` or the raw `$bytes` backing) and passes the `(ptr, len)` slice.
	HostWrite,
	/// A unary float math host import (log/exp/sin/cos): `(f64) -> f64`. The
	/// box/unbox to `$float` happens in wasm, so the host stays a bare libm call.
	F64Unary,
	/// `random-int`: `() -> i64` (the i64 crosses to/from the host as a JS BigInt).
	RngI64,
	/// `random-float`: `() -> f64`.
	RngF64,
	/// `random-int-range`: `(i64 lo, i64 hi) -> i64`.
	RngRange,
	/// `random-bytes`: `(i32 n, i32 dst, i32 cap) -> i32 len` — fill `n` random bytes
	/// into scratch at `dst` (overflow stashed, like the io reads).
	RngBytes,
	/// The scratch bump allocator `__alloc(i32 n) -> i32 ptr` — reserve `n` bytes in
	/// the exported linear memory (growing it as needed), return the start offset.
	MarshalAlloc,
	/// `__store_bytes((ref $bytes) b, i32 ptr) -> ()` — copy a GC `$bytes` array into
	/// scratch at `ptr` (the wasm→host byte-payload primitive).
	MarshalStore,
	/// `__load_bytes(i32 ptr, i32 len) -> (ref $bytes)` — copy `len` scratch bytes at
	/// `ptr` into a fresh GC `$bytes` array (the host→wasm byte-payload primitive).
	MarshalLoad,
	/// `__send_bytes((ref $bytes)) -> i32 len` — reset the bump cursor and copy a GC
	/// `$bytes` into scratch at offset 0, returning its length (the single-payload
	/// convenience the writer emit sites + the `print`-as-value wrapper share).
	MarshalSend,
	/// A `std/sys/io` host import with two i32 args → one i32 result: `(i32, i32) -> i32`.
	/// Covers the stdin reads + `io-last-error` (`(dst, cap) -> len`), `delete`/`mkdir`
	/// (`(path, plen) -> status`), and `exists`/`is-dir` (`(path, plen) -> bool`).
	Io2,
	/// A `std/sys/io` host import with four i32 args → one i32 result: `(i32,i32,i32,i32)
	/// -> i32`. Covers the path reads (`(path, plen, dst, cap) -> len`) and the file
	/// writers (`(path, plen, data, dlen) -> status`).
	Io4,
	/// `time-sleep(i64 nanos) -> ()` — the host blocks for the duration.
	TimeSleep,
	/// `time-parse(fp, fl, ip, il, dst) -> i32 status` — strtime-parse two scratch
	/// strings; on ok the host writes the i64 nanos to `dst` (read back by `emit`).
	TimeParse,
	/// `__io_copyout(i32 dst) -> ()` — drain the host's read stash into scratch at
	/// `dst` (the overflow path: a read whose bytes didn't fit the caller's first cap).
	IoCopyout,
	/// `__read_names(i32 ptr, i32 len) -> value` — split a NUL-terminated name blob in
	/// scratch into a `$list` of `$str` (the `io.read-dir` host return shape).
	MarshalReadNames,
	/// `__entry_error((ref null eq) value) -> i32 len` — inspect `_entry`'s return for a
	/// `result.err e`, rendering `e` into scratch and returning its length, or `-1` if
	/// the return is not an error. Lets the host detect a program-level failure without
	/// reflecting the GC value (it shuttles the opaque ref back in).
	EntryError,
	/// A `wire` FNV mixer over a value: `(i64 hash, ref $value) -> i64`. Used by
	/// both the recursive schema fingerprint and the string mixer.
	WireMixVal,
	/// The `wire` length mixer: `(i64 hash, i64 n) -> i64` (mixes `n`'s LE bytes).
	WireMixLen,
	/// The codec's byte sink: `(i32 byte) -> ()` (appends to the encode buffer).
	WirePush,
	/// The codec's varint sink: `(i64 v) -> ()` (LEB128 into the encode buffer).
	WireUvarint,
	/// The recursive encoder: `(ref $value schema, ref $value val) -> ()`.
	WireEnc,
	/// The decode byte source: `() -> i32` (reads one input byte / sets `g_err`).
	WireReadByte,
	/// The decode varint source: `() -> i64` (reads a LEB128 varint / sets `g_err`).
	WireReadVarint,
	/// A `std/sys/net` op returning `(i32 status, i32 n)` from two i32 args:
	/// `(i32, i32) -> (i32, i32)`. `net-listen`/`net-connect` (`(addr_ptr, alen) ->
	/// (status, socket-id)`) and `net-accept` (`(fid, listener-id) -> (status, conn-id)`).
	NetListen,
	/// `net-close`: `(i32 id) -> i32 status`.
	NetClose,
	/// `net-local-addr`: `(i32 id, i32 dst, i32 cap) -> (i32 status, i32 len)` (the
	/// address string is written into scratch at `dst`).
	NetLocalAddr,
	/// `net-read`/`net-write`: `(i32 fid, i32 conn, i32 ptr, i32 len_or_cap) -> (i32
	/// status, i32 n)`. read writes ≤ `cap` bytes into scratch (returns `len`); write
	/// reads `len` bytes from scratch (returns the count written).
	NetRW,
	/// The reactor block step: `io-poll(i64 deadline) -> i32 woken-fid` (shared by net +
	/// offload).
	NetPoll,
	/// Drop a reactor registration: `io-unwatch(i32 fid) -> ()`.
	NetUnwatch,
	/// A `BlockingPool` offload op with an i64 arg: `(i32 fid, i64 arg) -> (i32 status, i32
	/// n)` — `offload-sleep` (arg = nanos). Submit-or-collect like the net read/write ops.
	OffloadSleep,
	/// The async `std/sys/fs` op `fs-op`: `(i32 fid, i32 op, i32 path_ptr, i32 path_len, i32
	/// data_ptr, i32 data_len, i32 dst, i32 cap) -> (i32 status, i32 len)`.
	OffloadOp,
	/// The async `std/sys/db` op `db-op`: `(i32 fid, i32 op, i32 conn, i32 sql_ptr, i32
	/// sql_len, i32 params_ptr, i32 params_len, i32 dst, i32 cap) -> (i32 status, i32 len)`.
	/// Like `OffloadOp` plus the connection id (an i31-friendly i32 — conn ids are small).
	DbOp,
	/// The sync `std/sys/fs` op `fs-op-sync`: `(i32 op, i32 path_ptr, i32 path_len, i32
	/// data_ptr, i32 data_len, i32 dst, i32 cap) -> i32 len`.
	FsOpSync,
	/// A `std/web/dom` node-producing import: `(i32 ptr, i32 len) -> externref`
	/// (`dom-create-element`/`dom-create-text` — a tag/text string in scratch →
	/// a fresh DOM node). The host returns the engine-managed `externref`; wasm
	/// boxes it into a `$extern` (`emit_dom`).
	DomMake,
	/// `dom-body() -> externref` — the document root node (no args).
	DomBody,
	/// `dom-append-child(externref parent, externref child) -> ()`. Both nodes
	/// are unboxed from their `$extern` wrappers before the call.
	DomAppend,
	/// `dom-set-attribute(externref node, i32 np, i32 nl, i32 vp, i32 vl) -> ()` —
	/// node handle + two scratch strings (name, value).
	DomSetAttr,
	/// `dom-set-text(externref node, i32 ptr, i32 len) -> ()` — set a node's text
	/// content from a scratch string.
	DomNodeStr,
	/// `dom-get-value(externref node, i32 dst, i32 cap) -> i32 len` — read an input
	/// node's `.value` into scratch at `dst` (≤ `cap`), returning the length (the
	/// probe-read shape, like the io reads).
	DomGetValue,
	/// `dom-add-listener(externref node, i32 np, i32 nl, i32 token) -> ()` — register
	/// a handler for the event named by the scratch string `(np, nl)`. `token` indexes
	/// the wasm-side handler registry; the host wires `addEventListener(name, e =>
	/// __dom_dispatch(token, e))`.
	DomListen,
	/// The exported event-dispatch entry `__dom_dispatch(i32 token, externref event)
	/// -> ()` — the host calls it when a registered DOM event fires; it looks up the
	/// handler closure by `token` and invokes it with the (boxed) event. Distinct from
	/// the host imports above: it is a wasm *export*, not an import.
	DomDispatch,
	/// A three-node op `(externref, externref, externref) -> ()` — `dom-replace-child`
	/// (parent, new, old) and `dom-insert-before` (parent, new, ref).
	DomExtern3,
	/// A one-node op `(externref) -> ()` — `event-prevent-default`.
	DomExtern1,
	/// `dom-child-at(externref node, i32 i) -> externref` — the node's i-th child
	/// (used by `render.hydrate` to walk a server-rendered tree by index).
	DomChildAt,
	/// `dom-set-timeout(i32 delay_ms, i32 token) -> ()` — ask the host to `setTimeout`
	/// a call to the exported `__browser_resume(token)` (the browser command runtime's
	/// real-timer source).
	DomSetTimeout,
	/// `dom-dev-store-set(i32 kp, i32 kl, i32 vp, i32 vl) -> ()` — the dev-only
	/// key/value store (backed by `localStorage`): two scratch strings (key, value),
	/// no node. Used by `pluma dev`'s HMR to carry the MVU model across a reload.
	DomDevStoreSet,
	/// `dom-dev-store-get(i32 kp, i32 kl, i32 dst, i32 cap) -> i32 len` — read the
	/// dev store value for a scratch-string key into scratch at `dst` (≤ `cap`),
	/// returning the length (the probe-read shape, like `dom-get-value`).
	DomDevStoreGet,
	/// A nullary thunk `() -> ()` — the browser command pump (`__browser_run`) and the
	/// timer-resume entry (`__browser_resume`).
	Thunk,
	/// `rpc-stream-open(i32 ptr, i32 len, i32 token) -> ()` — ask the host to start the
	/// `fetch` for a browser RPC subscription, given the marshalled request string and
	/// the channel token (`std/web/stream`).
	RpcStreamOpen,
}

/// A registered record *shape*: the WasmGC struct type interned for a distinct
/// name-sorted field set. Returned by `FuncTypes::intern_shape`. The struct is a
/// subtype of `$value` laid out `{ i32 tag, i32 shape_id, f0..fk }`, fields in the
/// shape's name-sorted order, each `(ref null $value)` (boxed). `shape_id` is a
/// dense 0-based id stamped into each instance so a generic boundary can recover
/// the shape; `field_count` is `k`.
#[derive(Clone, Copy)]
pub struct ShapeInfo {
	pub type_idx: u32,
	pub shape_id: u32,
}

/// One entry in `FuncTypes::pending`: either an interned function type or a
/// record-shape struct (carrying its field count for `encode`). Both share the
/// type-index space starting at `T_FIRST_FUNC`, assigned in interning order.
enum Pending {
	Func(FuncKind),
	Shape(u32),
	/// A monomorphized Pluma function type: a leading boxed `env` param, then one
	/// param per recorded `Repr` (an unboxed i64/f64/i32 where the function takes a
	/// concrete scalar), and the result `Repr`. Used for eligible direct-only
	/// functions so a hot numeric callee passes/returns raw scalars instead of
	/// boxing at every call boundary.
	Sig(Vec<Repr>, Repr),
}

pub struct FuncTypes {
	keys: std::collections::HashMap<FuncKind, u32>,
	pending: Vec<Pending>,
	/// name-sorted field set -> its interned struct info (dedup + lookup).
	shape_keys: std::collections::HashMap<Vec<String>, ShapeInfo>,
	/// Count of distinct shapes interned so far — assigns the next `shape_id`.
	shape_count: u32,
	/// `(param reprs, ret repr)` -> its interned function-type index (dedup).
	sig_keys: std::collections::HashMap<(Vec<Repr>, Repr), u32>,
}

impl FuncTypes {
	pub fn new() -> Self {
		Self {
			keys: std::collections::HashMap::new(),
			pending: Vec::new(),
			shape_keys: std::collections::HashMap::new(),
			shape_count: 0,
			sig_keys: std::collections::HashMap::new(),
		}
	}

	/// The type index for a monomorphized Pluma function with the given param/ret
	/// reprs: `(env, p0..pn) -> ret`, each scalar `Repr` an unboxed i64/f64/i32 and
	/// each `Boxed` a `$value` ref (so an all-`Boxed` signature is layout-identical
	/// to `for_arity`, but interned separately). Idempotent on a repeated signature.
	pub fn for_signature(&mut self, params: &[Repr], ret: Repr) -> u32 {
		let key = (params.to_vec(), ret);
		if let Some(&i) = self.sig_keys.get(&key) {
			return i;
		}
		let idx = T_FIRST_FUNC + self.pending.len() as u32;
		self.sig_keys.insert(key, idx);
		self.pending.push(Pending::Sig(params.to_vec(), ret));
		idx
	}

	fn intern(&mut self, k: FuncKind) -> u32 {
		if let Some(&i) = self.keys.get(&k) {
			return i;
		}
		let idx = T_FIRST_FUNC + self.pending.len() as u32;
		self.keys.insert(k, idx);
		self.pending.push(Pending::Func(k));
		idx
	}

	/// Intern the nominal struct type for a record *shape* (a name-sorted field
	/// set). Idempotent: the same field set always maps to the same struct type and
	/// `shape_id`. The fields must already be name-sorted (matching `MakeRecord`).
	pub fn intern_shape(&mut self, fields: &[String]) -> ShapeInfo {
		if let Some(info) = self.shape_keys.get(fields) {
			return *info;
		}
		let type_idx = T_FIRST_FUNC + self.pending.len() as u32;
		let shape_id = self.shape_count;
		let info = ShapeInfo { type_idx, shape_id };
		self.shape_keys.insert(fields.to_vec(), info);
		self.shape_count += 1;
		self.pending.push(Pending::Shape(fields.len() as u32));
		info
	}

	/// The type index for a Pluma function of the given arity (boxed in/out).
	pub fn for_arity(&mut self, arity: usize) -> u32 {
		self.intern(FuncKind::Pluma(arity))
	}

	/// The type index for a host import taking `arity` `anyref` args and either
	/// returning a boxed value (`returns_value`) or nothing.
	pub fn for_host(&mut self, arity: usize, returns_value: bool) -> u32 {
		self.intern(FuncKind::Host(arity, returns_value))
	}

	/// The type index for the structural-equality helper `(value, value) -> i32`.
	pub fn for_eq(&mut self) -> u32 {
		self.intern(FuncKind::Eq)
	}

	/// `__variant_payload(value) -> valarray`.
	pub fn for_variant_payload(&mut self) -> u32 {
		self.intern(FuncKind::VariantPayload)
	}

	/// `__variant_from_array(i32, value, valarray) -> value`.
	pub fn for_variant_from_array(&mut self) -> u32 {
		self.intern(FuncKind::VariantFromArray)
	}

	/// The type index for a runtime helper: `n` boxed args -> boxed value.
	pub fn for_helper(&mut self, n: usize) -> u32 {
		self.intern(FuncKind::Helper(n))
	}

	/// The type index for the array-concat helper: `(valarray, valarray) -> valarray`.
	pub fn for_arrconcat(&mut self) -> u32 {
		self.intern(FuncKind::ArrConcat)
	}

	/// The type index for the bytes-concat helper: `(bytes, bytes) -> bytes`.
	pub fn for_bytesconcat(&mut self) -> u32 {
		self.intern(FuncKind::BytesConcat)
	}

	/// The type index for the float-format host import: `(f64, i32, i32) -> i32`.
	pub fn for_float_to_str(&mut self) -> u32 {
		self.intern(FuncKind::FloatToStr)
	}

	/// The type index for a byte-payload writer host import: `(i32, i32) -> ()`.
	pub fn for_host_write(&mut self) -> u32 {
		self.intern(FuncKind::HostWrite)
	}

	/// The type index for a unary float math host import: `(f64) -> f64`.
	pub fn for_f64_unary(&mut self) -> u32 {
		self.intern(FuncKind::F64Unary)
	}

	/// `random-int`: `() -> i64`.
	pub fn for_rng_i64(&mut self) -> u32 {
		self.intern(FuncKind::RngI64)
	}

	/// `random-float`: `() -> f64`.
	pub fn for_rng_f64(&mut self) -> u32 {
		self.intern(FuncKind::RngF64)
	}

	/// `random-int-range`: `(i64, i64) -> i64`.
	pub fn for_rng_range(&mut self) -> u32 {
		self.intern(FuncKind::RngRange)
	}

	/// `random-bytes`: `(i32, i32, i32) -> i32`.
	pub fn for_rng_bytes(&mut self) -> u32 {
		self.intern(FuncKind::RngBytes)
	}

	/// The scratch bump allocator `__alloc(i32) -> i32`.
	pub fn for_marshal_alloc(&mut self) -> u32 {
		self.intern(FuncKind::MarshalAlloc)
	}

	/// The byte-store primitive `__store_bytes((ref $bytes), i32) -> ()`.
	pub fn for_marshal_store(&mut self) -> u32 {
		self.intern(FuncKind::MarshalStore)
	}

	/// The byte-load primitive `__load_bytes(i32, i32) -> (ref $bytes)`.
	pub fn for_marshal_load(&mut self) -> u32 {
		self.intern(FuncKind::MarshalLoad)
	}

	/// The single-payload send primitive `__send_bytes((ref $bytes)) -> i32`.
	pub fn for_marshal_send(&mut self) -> u32 {
		self.intern(FuncKind::MarshalSend)
	}

	/// A two-arg `std/sys/io` host import: `(i32, i32) -> i32`.
	pub fn for_io2(&mut self) -> u32 {
		self.intern(FuncKind::Io2)
	}

	/// A four-arg `std/sys/io` host import: `(i32, i32, i32, i32) -> i32`.
	pub fn for_io4(&mut self) -> u32 {
		self.intern(FuncKind::Io4)
	}

	/// `time-sleep(i64) -> ()`.
	pub fn for_time_sleep(&mut self) -> u32 {
		self.intern(FuncKind::TimeSleep)
	}

	/// `time-parse(i32, i32, i32, i32, i32) -> i32`.
	pub fn for_time_parse(&mut self) -> u32 {
		self.intern(FuncKind::TimeParse)
	}

	/// The read-stash drain host import `__io_copyout(i32) -> ()`.
	pub fn for_io_copyout(&mut self) -> u32 {
		self.intern(FuncKind::IoCopyout)
	}

	/// The read-dir splitter `__read_names(i32, i32) -> value`.
	pub fn for_marshal_read_names(&mut self) -> u32 {
		self.intern(FuncKind::MarshalReadNames)
	}

	/// The entry-error probe `__entry_error((ref null eq)) -> i32`.
	pub fn for_entry_error(&mut self) -> u32 {
		self.intern(FuncKind::EntryError)
	}

	/// `net-listen`/`net-connect`/`net-accept`: `(i32, i32) -> (i32, i32)`.
	pub fn for_net_listen(&mut self) -> u32 {
		self.intern(FuncKind::NetListen)
	}

	/// `net-close`: `(i32) -> i32`.
	pub fn for_net_close(&mut self) -> u32 {
		self.intern(FuncKind::NetClose)
	}

	/// `net-local-addr`: `(i32, i32, i32) -> (i32, i32)`.
	pub fn for_net_local_addr(&mut self) -> u32 {
		self.intern(FuncKind::NetLocalAddr)
	}

	/// `net-read`/`net-write`: `(i32, i32, i32, i32) -> (i32, i32)`.
	pub fn for_net_rw(&mut self) -> u32 {
		self.intern(FuncKind::NetRW)
	}

	/// `net-poll`: `(i64 deadline) -> i32`.
	pub fn for_net_poll(&mut self) -> u32 {
		self.intern(FuncKind::NetPoll)
	}

	/// `net-unwatch`: `(i32 fid) -> ()`.
	pub fn for_net_unwatch(&mut self) -> u32 {
		self.intern(FuncKind::NetUnwatch)
	}

	/// `offload-sleep` and kin: `(i32 fid, i64 arg) -> (i32 status, i32 n)`.
	pub fn for_offload_sleep(&mut self) -> u32 {
		self.intern(FuncKind::OffloadSleep)
	}

	/// `fs-op`: `(i32 fid, i32 op, i32 pp, i32 pl, i32 dp, i32 dl, i32 dst, i32 cap) -> (i32
	/// status, i32 len)`.
	pub fn for_offload_op(&mut self) -> u32 {
		self.intern(FuncKind::OffloadOp)
	}

	/// `fs-op-sync`: `(i32 op, i32 pp, i32 pl, i32 dp, i32 dl, i32 dst, i32 cap) -> i32 len`.
	pub fn for_fs_op_sync(&mut self) -> u32 {
		self.intern(FuncKind::FsOpSync)
	}

	/// `db-op`: `(i32 fid, i32 op, i32 conn, i32 sp, i32 sl, i32 pp, i32 pl, i32 dst, i32
	/// cap) -> (i32 status, i32 len)`.
	pub fn for_db_op(&mut self) -> u32 {
		self.intern(FuncKind::DbOp)
	}

	/// The type index for a `wire` value mixer: `(i64, ref $value) -> i64`.
	pub fn for_wire_mix_val(&mut self) -> u32 {
		self.intern(FuncKind::WireMixVal)
	}

	/// The type index for the `wire` length mixer: `(i64, i64) -> i64`.
	pub fn for_wire_mix_len(&mut self) -> u32 {
		self.intern(FuncKind::WireMixLen)
	}

	/// The type index for the codec byte sink: `(i32) -> ()`.
	pub fn for_wire_push(&mut self) -> u32 {
		self.intern(FuncKind::WirePush)
	}

	/// The type index for the codec varint sink: `(i64) -> ()`.
	pub fn for_wire_uvarint(&mut self) -> u32 {
		self.intern(FuncKind::WireUvarint)
	}

	/// The type index for the recursive encoder: `(value, value) -> ()`.
	pub fn for_wire_enc(&mut self) -> u32 {
		self.intern(FuncKind::WireEnc)
	}

	/// The type index for the decode byte source: `() -> i32`.
	pub fn for_wire_rbyte(&mut self) -> u32 {
		self.intern(FuncKind::WireReadByte)
	}

	/// The type index for the decode varint source: `() -> i64`.
	pub fn for_wire_ruvarint(&mut self) -> u32 {
		self.intern(FuncKind::WireReadVarint)
	}

	/// `std/web/dom` node-producing import type: `(i32, i32) -> externref`.
	pub fn for_dom_make(&mut self) -> u32 {
		self.intern(FuncKind::DomMake)
	}

	/// `dom-body`: `() -> externref`.
	pub fn for_dom_body(&mut self) -> u32 {
		self.intern(FuncKind::DomBody)
	}

	/// `dom-append-child`: `(externref, externref) -> ()`.
	pub fn for_dom_append(&mut self) -> u32 {
		self.intern(FuncKind::DomAppend)
	}

	/// `dom-set-attribute`: `(externref, i32, i32, i32, i32) -> ()`.
	pub fn for_dom_set_attr(&mut self) -> u32 {
		self.intern(FuncKind::DomSetAttr)
	}

	/// `dom-set-text`: `(externref, i32, i32) -> ()`.
	pub fn for_dom_node_str(&mut self) -> u32 {
		self.intern(FuncKind::DomNodeStr)
	}

	/// `dom-get-value`: `(externref, i32, i32) -> i32`.
	pub fn for_dom_get_value(&mut self) -> u32 {
		self.intern(FuncKind::DomGetValue)
	}

	/// `dom-add-listener`: `(externref, i32, i32, i32) -> ()`.
	pub fn for_dom_listen(&mut self) -> u32 {
		self.intern(FuncKind::DomListen)
	}

	/// The exported `__dom_dispatch(i32, externref) -> ()` entry type.
	pub fn for_dom_dispatch(&mut self) -> u32 {
		self.intern(FuncKind::DomDispatch)
	}

	/// A three-node op `(externref, externref, externref) -> ()`.
	pub fn for_dom_extern3(&mut self) -> u32 {
		self.intern(FuncKind::DomExtern3)
	}

	/// A one-node op `(externref) -> ()`.
	pub fn for_dom_extern1(&mut self) -> u32 {
		self.intern(FuncKind::DomExtern1)
	}

	/// `dom-child-at`: `(externref, i32) -> externref`.
	pub fn for_dom_child_at(&mut self) -> u32 {
		self.intern(FuncKind::DomChildAt)
	}

	/// `dom-set-timeout`: `(i32, i32) -> ()`.
	pub fn for_dom_set_timeout(&mut self) -> u32 {
		self.intern(FuncKind::DomSetTimeout)
	}

	/// `dom-dev-store-set`: `(i32, i32, i32, i32) -> ()` — two scratch strings.
	pub fn for_dom_dev_store_set(&mut self) -> u32 {
		self.intern(FuncKind::DomDevStoreSet)
	}

	/// `dom-dev-store-get`: `(i32, i32, i32, i32) -> i32` — string key in, probe-read out.
	pub fn for_dom_dev_store_get(&mut self) -> u32 {
		self.intern(FuncKind::DomDevStoreGet)
	}

	/// A nullary thunk `() -> ()`.
	pub fn for_thunk(&mut self) -> u32 {
		self.intern(FuncKind::Thunk)
	}

	/// `rpc-stream-open`: `(i32, i32, i32) -> ()`.
	pub fn for_rpc_stream_open(&mut self) -> u32 {
		self.intern(FuncKind::RpcStreamOpen)
	}

	/// Encode the full type section: the fixed `$value` prefix, then every
	/// interned function type in index order.
	pub fn encode(&self) -> TypeSection {
		let mut types = TypeSection::new();
		// 0: $value — the open, subtypeable boxed supertype.
		types.ty().subtype(&struct_subtype(
			None,
			vec![val_field(ValType::I32, false)],
			false,
		));
		// 1: $bytes — array (mut i8).
		types.ty().subtype(&SubType {
			is_final: true,
			supertype_idx: None,
			composite_type: CompositeType {
				inner: CompositeInnerType::Array(wasm_encoder::ArrayType(FieldType {
					element_type: StorageType::I8,
					mutable: true,
				})),
				shared: false,
				descriptor: None,
				describes: None,
			},
		});
		// 2..6: scalar/heap subtypes of $value.
		let scalar = |payload: ValType| {
			struct_subtype(
				Some(T_VALUE),
				vec![val_field(ValType::I32, false), val_field(payload, false)],
				true,
			)
		};
		types.ty().subtype(&scalar(ValType::I64)); // 2 $int
		types.ty().subtype(&scalar(ValType::F64)); // 3 $float
		types.ty().subtype(&scalar(ValType::I32)); // 4 $bool
		types.ty().subtype(&scalar(bytes_ref())); // 5 $str
		// 6 $valarray — array (mut (ref null $value)).
		types.ty().subtype(&SubType {
			is_final: true,
			supertype_idx: None,
			composite_type: CompositeType {
				inner: CompositeInnerType::Array(wasm_encoder::ArrayType(val_field(value_ref(), true))),
				shared: false,
				descriptor: None,
				describes: None,
			},
		});
		// 7 $closure — { tag, i32 fn_index, (ref $valarray) captures }.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 8 $variant — { tag, i32 variant_tag, (ref $str) display-name, i32 arity,
		// p0, p1, (ref null $valarray) rest }. The payload is stored *inline*: arity
		// ≤ 2 keeps its elements in `p0`/`p1` (with `rest` null), so `tree.node l r`,
		// `option`, and `result` allocate just this one struct — no separate payload
		// array. Arity ≥ 3 (rare) stores the whole payload in `rest` (and leaves
		// `p0`/`p1` null), so no element ever needs copying. Generic consumers that
		// want the payload as a uniform `$valarray` (print/eq/wire) call
		// `__variant_payload`, which returns `rest` directly when present or
		// materializes a small array from `p0`/`p1`; the hot construct/match/payload
		// paths, where the arity is statically known, read the inline slots.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(value_ref(), false),
				val_field(ValType::I32, false),
				val_field(value_ref(), false),
				val_field(value_ref(), false),
				val_field(valarray_ref_null(), false),
			],
			true,
		));
		// 9 $ctor — a partial variant constructor: { tag, i32 variant_tag, i32 arity }.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
			],
			true,
		));
		// 10 $methoddict — { tag, (ref $valarray) methods } (positional method values).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 11 $tuple — { tag, (ref $valarray) elems } (fixed arity).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 12 $list — { tag, (mut ref $valarray) elems, (mut i32) length }. The
		// logical length can be < the backing array's capacity: `list.push`
		// appends in place, growing/swapping `elems` (mutable) and bumping
		// `length` (mutable) only when full. Every length read uses field 2, NOT
		// `array.len(elems)` (which is the capacity).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(valarray_ref(), true),
				val_field(ValType::I32, true),
			],
			true,
		));
		// 13 $record — { tag, (ref $valarray) names, (ref $valarray) values } (name-sorted).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 14 $ref — { tag, (mut ref null $value) cell }. The cell field is mutable
		// (the whole point of a `ref`); identity is the struct reference itself.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![val_field(ValType::I32, false), val_field(value_ref(), true)],
			true,
		));
		// 15 $dict — { tag, (mut ref null $value) root, (mut i32) size }.
		// An immutable persistent hash-array-mapped trie (see `helpers/dict.rs`):
		// `root` is the top `$cnode` (or null when empty), `size` caches the live entry
		// count for O(1) `dict.size`. `insert`/`remove` path-copy and return a new
		// `$dict`; the fields are mutable only so the internal transient builder can
		// write in place before freezing.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(value_ref(), true),
				val_field(ValType::I32, true),
			],
			true,
		));
		// 16 $task — { tag, i32 kind, (ref $valarray) payload }. A cold async task;
		// `kind` is the `TaskRepr` discriminant, `payload` its components.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, false),
				val_field(valarray_ref(), false),
			],
			true,
		));
		// 17 $dentry — one entry of a `$dict`'s `order` list: { tag, key, (mut) value,
		// i64 hash }. A subtype of `$value` so it sits in the `order` `$list`'s
		// `$valarray` like any value. `value` is mutable (an insert of an existing key
		// overwrites it in place); `hash` caches `__hash(key)` so resize rehashes and
		// probes compare the full hash before the costlier `__eq`. `tag` is an unused
		// sentinel (a `$dentry` never escapes to tag-inspecting code).
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(value_ref(), false),
				val_field(value_ref(), true),
				val_field(ValType::I64, false),
			],
			true,
		));
		// 18 $extern — { tag, (ref null extern) handle }. Boxes an engine-managed
		// `externref` host resource (DOM node / fetch response) as a `$value` subtype,
		// so a handle flows through Pluma code like any value. The field is an
		// `externref` (not an `eqref`), but the wrapper struct *is* an `eqref`, so it
		// boxes/stores/pattern-matches normally; identity is the struct reference
		// (`ref.eq`, like `$ref`). Only the Web target's imports build one.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(extern_ref(), false),
			],
			true,
		));
		// 19 $cnode — a persistent-dict trie node: { tag, i32 dataMap, i32 nodeMap,
		// (mut ref $valarray) entries, (mut ref $valarray) children, (mut ref null
		// $value) edit }. `entries` holds `$dentry` leaves, `children` holds child
		// `$cnode`s (both ride the shared `$valarray`, cast on read); the bitmaps give
		// CHAMP slot indexing via `popcnt`. `edit` is the transient owner token (null
		// when frozen). A `$value` subtype with a distinct `tag` so the trie walk tells
		// a leaf from a sub-node by field 0; never escapes to user code.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(ValType::I32, true),
				val_field(ValType::I32, true),
				val_field(valarray_ref(), true),
				val_field(valarray_ref(), true),
				val_field(value_ref(), true),
			],
			true,
		));
		// 20 $local — { tag, (ref null $value) default }. A task-local cell: the struct
		// reference is the cell's identity (`ref.eq`, like `$ref`); `default` is what
		// `local.get` returns outside any `local.with` binding. Immutable.
		types.ty().subtype(&struct_subtype(
			Some(T_VALUE),
			vec![
				val_field(ValType::I32, false),
				val_field(value_ref(), false),
			],
			true,
		));
		// Interned function types + record-shape structs, in index order. A Pluma
		// function takes an implicit closure-environment param first (`env`, the
		// `$closure` ref or null for a capture-free direct call), then its `arity`
		// boxed params. A record shape is a `$value` subtype `{ tag, shape_id,
		// f0..fk }` with `k` boxed fields, in the shape's name-sorted order.
		for p in &self.pending {
			let k = match p {
				Pending::Func(k) => *k,
				Pending::Sig(params, ret) => {
					// `(env, p0..pn) -> ret`: a leading boxed env (direct calls pass null),
					// then each param's repr, then the result repr.
					let ps: Vec<ValType> = std::iter::once(value_ref())
						.chain(params.iter().map(|r| repr_valtype(*r)))
						.collect();
					types.ty().function(ps, [repr_valtype(*ret)]);
					continue;
				}
				Pending::Shape(n) => {
					let mut fields = vec![
						val_field(ValType::I32, false),
						val_field(ValType::I32, false),
					];
					fields.extend(std::iter::repeat(val_field(value_ref(), false)).take(*n as usize));
					types
						.ty()
						.subtype(&struct_subtype(Some(T_VALUE), fields, true));
					continue;
				}
			};
			let (param_ty, count, results): (ValType, usize, Vec<ValType>) = match k {
				FuncKind::Pluma(arity) => (value_ref(), arity + 1, vec![value_ref()]),
				FuncKind::Host(arity, returns_value) => (
					any_ref(),
					arity,
					if returns_value {
						vec![value_ref()]
					} else {
						vec![]
					},
				),
				FuncKind::Eq => (value_ref(), 2, vec![ValType::I32]),
				FuncKind::VariantPayload => {
					types.ty().function([value_ref()], [valarray_ref()]);
					continue;
				}
				FuncKind::VariantFromArray => {
					types
						.ty()
						.function([ValType::I32, value_ref(), valarray_ref()], [value_ref()]);
					continue;
				}
				FuncKind::Helper(n) => (value_ref(), n, vec![value_ref()]),
				FuncKind::ArrConcat => (valarray_ref(), 2, vec![valarray_ref()]),
				FuncKind::BytesConcat => (bytes_ref(), 2, vec![bytes_ref()]),
				// Heterogeneous params — built directly below rather than via `param_ty`.
				FuncKind::FloatToStr => {
					types
						.ty()
						.function([ValType::F64, ValType::I32, ValType::I32], [ValType::I32]);
					continue;
				}
				FuncKind::HostWrite => {
					types.ty().function([ValType::I32, ValType::I32], []);
					continue;
				}
				FuncKind::F64Unary => {
					types.ty().function([ValType::F64], [ValType::F64]);
					continue;
				}
				FuncKind::RngI64 => {
					types.ty().function([], [ValType::I64]);
					continue;
				}
				FuncKind::RngF64 => {
					types.ty().function([], [ValType::F64]);
					continue;
				}
				FuncKind::RngRange => {
					types
						.ty()
						.function([ValType::I64, ValType::I64], [ValType::I64]);
					continue;
				}
				FuncKind::RngBytes => {
					types
						.ty()
						.function([ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);
					continue;
				}
				// Marshalling-helper types — heterogeneous, built directly.
				FuncKind::MarshalAlloc => {
					types.ty().function([ValType::I32], [ValType::I32]);
					continue;
				}
				FuncKind::MarshalStore => {
					types.ty().function([bytes_ref(), ValType::I32], []);
					continue;
				}
				FuncKind::MarshalLoad => {
					types
						.ty()
						.function([ValType::I32, ValType::I32], [bytes_ref()]);
					continue;
				}
				FuncKind::MarshalSend => {
					types.ty().function([bytes_ref()], [ValType::I32]);
					continue;
				}
				FuncKind::Io2 => {
					types
						.ty()
						.function([ValType::I32, ValType::I32], [ValType::I32]);
					continue;
				}
				FuncKind::Io4 => {
					types.ty().function(
						[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
						[ValType::I32],
					);
					continue;
				}
				FuncKind::TimeSleep => {
					types.ty().function([ValType::I64], []);
					continue;
				}
				FuncKind::TimeParse => {
					types.ty().function(
						[
							ValType::I32,
							ValType::I32,
							ValType::I32,
							ValType::I32,
							ValType::I32,
						],
						[ValType::I32],
					);
					continue;
				}
				FuncKind::IoCopyout => {
					types.ty().function([ValType::I32], []);
					continue;
				}
				FuncKind::MarshalReadNames => {
					types
						.ty()
						.function([ValType::I32, ValType::I32], [value_ref()]);
					continue;
				}
				FuncKind::EntryError => {
					types.ty().function([value_ref()], [ValType::I32]);
					continue;
				}
				// std/sys/net host imports. Each fallible op returns a
				// `(status:i32, n:i32)` pair — `n` is a socket id / byte count / read
				// length; byte payloads (addr, data, the read result) cross via scratch.
				FuncKind::NetListen => {
					types
						.ty()
						.function([ValType::I32, ValType::I32], [ValType::I32, ValType::I32]);
					continue;
				}
				FuncKind::NetClose => {
					types.ty().function([ValType::I32], [ValType::I32]);
					continue;
				}
				FuncKind::NetLocalAddr => {
					types.ty().function(
						[ValType::I32, ValType::I32, ValType::I32],
						[ValType::I32, ValType::I32],
					);
					continue;
				}
				FuncKind::NetRW => {
					types.ty().function(
						[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
						[ValType::I32, ValType::I32],
					);
					continue;
				}
				FuncKind::NetPoll => {
					types.ty().function([ValType::I64], [ValType::I32]);
					continue;
				}
				FuncKind::NetUnwatch => {
					types.ty().function([ValType::I32], []);
					continue;
				}
				FuncKind::OffloadSleep => {
					types
						.ty()
						.function([ValType::I32, ValType::I64], [ValType::I32, ValType::I32]);
					continue;
				}
				FuncKind::OffloadOp => {
					types
						.ty()
						.function([ValType::I32; 8], [ValType::I32, ValType::I32]);
					continue;
				}
				FuncKind::DbOp => {
					types
						.ty()
						.function([ValType::I32; 9], [ValType::I32, ValType::I32]);
					continue;
				}
				FuncKind::FsOpSync => {
					types.ty().function([ValType::I32; 7], [ValType::I32]);
					continue;
				}
				FuncKind::WireMixVal => {
					types
						.ty()
						.function([ValType::I64, value_ref()], [ValType::I64]);
					continue;
				}
				FuncKind::WireMixLen => {
					types
						.ty()
						.function([ValType::I64, ValType::I64], [ValType::I64]);
					continue;
				}
				FuncKind::WirePush => {
					types.ty().function([ValType::I32], []);
					continue;
				}
				FuncKind::WireUvarint => {
					types.ty().function([ValType::I64], []);
					continue;
				}
				FuncKind::WireEnc => {
					types.ty().function([value_ref(), value_ref()], []);
					continue;
				}
				FuncKind::WireReadByte => {
					types.ty().function([], [ValType::I32]);
					continue;
				}
				FuncKind::WireReadVarint => {
					types.ty().function([], [ValType::I64]);
					continue;
				}
				// std/web/dom host imports (the Web target) — node handles cross as
				// `externref`, strings as scratch `(ptr, len)`. The one externref-
				// returning shape is `DomMake`/`DomBody`.
				FuncKind::DomMake => {
					types
						.ty()
						.function([ValType::I32, ValType::I32], [extern_ref()]);
					continue;
				}
				FuncKind::DomBody => {
					types.ty().function([], [extern_ref()]);
					continue;
				}
				FuncKind::DomAppend => {
					types.ty().function([extern_ref(), extern_ref()], []);
					continue;
				}
				FuncKind::DomSetAttr => {
					types.ty().function(
						[
							extern_ref(),
							ValType::I32,
							ValType::I32,
							ValType::I32,
							ValType::I32,
						],
						[],
					);
					continue;
				}
				FuncKind::DomNodeStr => {
					types
						.ty()
						.function([extern_ref(), ValType::I32, ValType::I32], []);
					continue;
				}
				FuncKind::DomGetValue => {
					types
						.ty()
						.function([extern_ref(), ValType::I32, ValType::I32], [ValType::I32]);
					continue;
				}
				FuncKind::DomListen => {
					types
						.ty()
						.function([extern_ref(), ValType::I32, ValType::I32, ValType::I32], []);
					continue;
				}
				FuncKind::DomDispatch => {
					types.ty().function([ValType::I32, extern_ref()], []);
					continue;
				}
				FuncKind::DomExtern3 => {
					types
						.ty()
						.function([extern_ref(), extern_ref(), extern_ref()], []);
					continue;
				}
				FuncKind::DomExtern1 => {
					types.ty().function([extern_ref()], []);
					continue;
				}
				FuncKind::DomChildAt => {
					types
						.ty()
						.function([extern_ref(), ValType::I32], [extern_ref()]);
					continue;
				}
				FuncKind::DomSetTimeout => {
					types.ty().function([ValType::I32, ValType::I32], []);
					continue;
				}
				FuncKind::DomDevStoreSet => {
					types
						.ty()
						.function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], []);
					continue;
				}
				FuncKind::DomDevStoreGet => {
					types.ty().function(
						[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
						[ValType::I32],
					);
					continue;
				}
				FuncKind::Thunk => {
					types.ty().function([], []);
					continue;
				}
				FuncKind::RpcStreamOpen => {
					types
						.ty()
						.function([ValType::I32, ValType::I32, ValType::I32], []);
					continue;
				}
			};
			let params = std::iter::repeat(param_ty).take(count);
			types.ty().function(params, results);
		}
		types
	}
}
