// The WasmGC runtime host. Instantiates an emitted module in wasmtime, supplies
// the `pluma.*` host imports (print/io/float_to_str/math/io-read) by reflecting
// the program's GC `$value` layout, and runs `_entry`. The `print` path mirrors
// `vm::Value`'s Display so wasm stdout is byte-identical to the VM oracle.
//
// Two front doors share one engine + one set of host imports:
//   - `run_wasm`/`run_entry` — **buffered** (stdout captured, stderr dropped,
//     stdin fed from a byte slice). The `conformance` crate's differential path.
//   - `run_streaming` — **process stdio** (stdout/stderr streamed live, stdin read
//     from the process). The `cli`'s `pluma run app.wasm` path.
// The only thing that differs is the `HostIo` sink behind `HostState`; the GC
// reflection and every host import are identical, so the conformance gate tests
// exactly the runtime the CLI ships.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::time::Duration;

use polling::{Event, Events, Poller};
use wasmtime::{
	AnyRef, ArrayRef, ArrayRefPre, ArrayType, AsContextMut, Caller, Config, Engine, ExternType,
	FuncType, Instance, Linker, Module, RootScope, Rooted, Store, StructRef, StructRefPre,
	StructType, Val, ValType,
};

/// A program's observable result: exit status + captured stdout. (`run_streaming`
/// returns an empty `stdout` — it streamed live to the process — and the caller
/// uses `status` for the exit code.)
#[derive(Clone, PartialEq, Eq)]
pub struct RunResult {
	pub status: String,
	pub stdout: String,
}

const TAG_NOTHING: i32 = 0;
const TAG_BOOL: i32 = 1;
const TAG_INT: i32 = 2;
const TAG_FLOAT: i32 = 3;
const TAG_STR: i32 = 4;
const TAG_DURATION: i32 = 5;
const TAG_VARIANT: i32 = 8;
const TAG_TUPLE: i32 = 11;
const TAG_LIST: i32 = 12;
const TAG_RECORD: i32 = 13;
const TAG_BYTES: i32 = 14;
const TAG_REF: i32 = 15;
const TAG_DICT: i32 = 16;

// --------------------------------------------------------------------------
// The stdio sink. `HostState` is non-generic (the host-import closures live in a
// `Linker<HostState>`); the buffered-vs-streaming choice is a trait object.
// --------------------------------------------------------------------------

/// Where the host's stdout/stderr go and where stdin comes from. The reads use the
/// VM's `read_line` semantics (line up to `\n`, trailing `\r` stripped, `None` at
/// EOF); `read_rest` drains the remainder.
pub trait HostIo {
	fn write_out(&mut self, bytes: &[u8]);
	fn write_err(&mut self, bytes: &[u8]);
	fn read_line(&mut self) -> Option<String>;
	fn read_rest(&mut self) -> Vec<u8>;
	/// The stdout collected so far, for the buffered (diff) path. Streaming impls
	/// return `""` (they wrote straight to the process).
	fn captured_stdout(&self) -> String {
		String::new()
	}
}

/// Captures stdout, drops stderr (the conformance differential compares stdout
/// only, mirroring `run_vm`), and reads stdin from a fixed byte buffer.
struct BufferedIo {
	out: Vec<u8>,
	stdin: Vec<u8>,
	stdin_pos: usize,
}

impl BufferedIo {
	fn new(stdin: &[u8]) -> Self {
		BufferedIo {
			out: Vec::new(),
			stdin: stdin.to_vec(),
			stdin_pos: 0,
		}
	}
}

impl HostIo for BufferedIo {
	fn write_out(&mut self, bytes: &[u8]) {
		self.out.extend_from_slice(bytes);
	}
	fn write_err(&mut self, _bytes: &[u8]) {}
	fn read_line(&mut self) -> Option<String> {
		read_line_from(&self.stdin, &mut self.stdin_pos)
	}
	fn read_rest(&mut self) -> Vec<u8> {
		let rest = self.stdin[self.stdin_pos..].to_vec();
		self.stdin_pos = self.stdin.len();
		rest
	}
	fn captured_stdout(&self) -> String {
		String::from_utf8_lossy(&self.out).into_owned()
	}
}

/// Streams stdout/stderr live to the process and reads stdin from it. Each write
/// flushes so output appears promptly (Phase 1 artifacts are short-lived; a
/// long-running net server in a later phase will revisit buffering).
struct StdioIo {
	stdin_buf: Vec<u8>,
	stdin_pos: usize,
	stdin_eof: bool,
}

impl StdioIo {
	fn new() -> Self {
		StdioIo {
			stdin_buf: Vec::new(),
			stdin_pos: 0,
			stdin_eof: false,
		}
	}
	/// Pull all of process stdin into the buffer once, on first read. (Phase 1
	/// reads are whole-input oriented; live line streaming is a later concern.)
	fn fill_stdin(&mut self) {
		if !self.stdin_eof {
			let _ = std::io::stdin().read_to_end(&mut self.stdin_buf);
			self.stdin_eof = true;
		}
	}
}

impl HostIo for StdioIo {
	fn write_out(&mut self, bytes: &[u8]) {
		let mut out = std::io::stdout();
		let _ = out.write_all(bytes);
		let _ = out.flush();
	}
	fn write_err(&mut self, bytes: &[u8]) {
		let mut err = std::io::stderr();
		let _ = err.write_all(bytes);
		let _ = err.flush();
	}
	fn read_line(&mut self) -> Option<String> {
		self.fill_stdin();
		read_line_from(&self.stdin_buf, &mut self.stdin_pos)
	}
	fn read_rest(&mut self) -> Vec<u8> {
		self.fill_stdin();
		let rest = self.stdin_buf[self.stdin_pos..].to_vec();
		self.stdin_pos = self.stdin_buf.len();
		rest
	}
}

/// Read one line from `buf` at `*pos` with the VM's `read_line` semantics: `None`
/// at EOF; otherwise the bytes up to the next `\n` (consumed), trailing `\r`
/// stripped.
fn read_line_from(buf: &[u8], pos: &mut usize) -> Option<String> {
	if *pos >= buf.len() {
		return None;
	}
	let start = *pos;
	let (end, next) = match buf[start..].iter().position(|&c| c == b'\n') {
		Some(rel) => (start + rel, start + rel + 1),
		None => (buf.len(), buf.len()),
	};
	let line_end = if end > start && buf[end - 1] == b'\r' {
		end - 1
	} else {
		end
	};
	let s = String::from_utf8_lossy(&buf[start..line_end]).into_owned();
	*pos = next;
	Some(s)
}

struct HostState {
	io: Box<dyn HostIo>,
	/// The `io.fail` abort message, stashed before the host traps so the runner can
	/// surface it as the program's `runtime error: <msg>` status.
	fail: Option<String>,
	/// The message the last failed `core.io` call stashed (errno-style); returned
	/// by the `io-last-error` import, which `__io_result` queries on the err path.
	last_error: String,
	/// The module's `$value` GC types, captured once from the witness the first io
	/// host import receives, so later calls build their returns without re-reflecting.
	gc_types: Option<GcTypes>,
	/// `core.net` runtime state: the socket table + the I/O reactor (the host-side
	/// analogue of `vm::net::NetState`).
	net: HostNet,
}

/// The module's GC type handles the io host imports need to build their `$value`
/// returns. Engine-scoped (store-independent), captured once from the universal
/// witness `[nothing, "", [], true]` every io import is passed (see `emit_io_witness`).
#[derive(Clone)]
struct GcTypes {
	value: StructType,
	str_: StructType,
	bytes: ArrayType,
	valarray: ArrayType,
	list: StructType,
	bool_: StructType,
}

/// Reflect the universal witness (a `$list [nothing, "", [], true]`) to recover the
/// module's `$value`/`$str`/`$bytes`/`$list`/`$valarray`/`$bool` types.
fn capture_gc_types(store: &mut impl AsContextMut, witness: &Val) -> GcTypes {
	let mut scope = RootScope::new(store);
	let list_ref = match witness {
		Val::AnyRef(Some(r)) => *r,
		o => panic!("io witness is not a ref: {o:?}"),
	};
	let list_struct = list_ref
		.as_struct(&mut scope)
		.expect("witness as_struct")
		.expect("witness is a $list");
	let list = list_struct.ty(&mut scope).expect("$list type");
	let elems = match list_struct.field(&mut scope, 1).expect("witness elems") {
		Val::AnyRef(Some(r)) => r
			.as_array(&mut scope)
			.expect("as_array")
			.expect("$valarray"),
		o => panic!("witness elems not an array: {o:?}"),
	};
	let valarray = elems.ty(&mut scope).expect("$valarray type");
	let struct_ty_at = |scope: &mut RootScope<_>, i: u32| -> StructType {
		match elems.get(&mut *scope, i).expect("witness elem") {
			Val::AnyRef(Some(r)) => r
				.as_struct(&mut *scope)
				.expect("elem as_struct")
				.expect("elem struct")
				.ty(&mut *scope)
				.expect("elem type"),
			o => panic!("witness elem {i} not a struct: {o:?}"),
		}
	};
	let value = struct_ty_at(&mut scope, 0); // nothing
	let str_ = struct_ty_at(&mut scope, 1); // ""
	let bytes = {
		let s = match elems.get(&mut scope, 1).expect("elem 1") {
			Val::AnyRef(Some(r)) => r.as_struct(&mut scope).unwrap().unwrap(),
			o => panic!("elem 1 not a struct: {o:?}"),
		};
		match s.field(&mut scope, 1).expect("$str bytes field") {
			Val::AnyRef(Some(r)) => r
				.as_array(&mut scope)
				.unwrap()
				.unwrap()
				.ty(&mut scope)
				.unwrap(),
			o => panic!("$str field 1 not an array: {o:?}"),
		}
	};
	let bool_ = struct_ty_at(&mut scope, 3); // true
	GcTypes {
		value,
		str_,
		bytes,
		valarray,
		list,
		bool_,
	}
}

/// Build a `$bytes` GC array of `data`.
fn build_bytes_array(store: &mut impl AsContextMut, gc: &GcTypes, data: &[u8]) -> Rooted<ArrayRef> {
	let pre = ArrayRefPre::new(&mut *store, gc.bytes.clone());
	let elems: Vec<Val> = data.iter().map(|&b| Val::I32(b as i32)).collect();
	ArrayRef::new_fixed(&mut *store, &pre, &elems).expect("build $bytes")
}

/// Build a `$str`/`$bytes`-shaped `$value` (`{tag, $bytes}`) for `tag` + `data`.
fn build_strlike(store: &mut impl AsContextMut, gc: &GcTypes, tag: i32, data: &[u8]) -> Val {
	let bytes = build_bytes_array(&mut *store, gc, data);
	let pre = StructRefPre::new(&mut *store, gc.str_.clone());
	let s = StructRef::new(
		&mut *store,
		&pre,
		&[Val::I32(tag), Val::AnyRef(Some(bytes.into()))],
	)
	.expect("build $str");
	Val::AnyRef(Some(s.to_anyref()))
}

/// Build a `nothing` `$value`.
fn build_nothing(store: &mut impl AsContextMut, gc: &GcTypes) -> Val {
	let pre = StructRefPre::new(&mut *store, gc.value.clone());
	let s = StructRef::new(&mut *store, &pre, &[Val::I32(TAG_NOTHING)]).expect("build nothing");
	Val::AnyRef(Some(s.to_anyref()))
}

/// Build a `$bool` `$value`.
fn build_bool(store: &mut impl AsContextMut, gc: &GcTypes, b: bool) -> Val {
	let pre = StructRefPre::new(&mut *store, gc.bool_.clone());
	let s = StructRef::new(&mut *store, &pre, &[Val::I32(TAG_BOOL), Val::I32(b as i32)])
		.expect("build $bool");
	Val::AnyRef(Some(s.to_anyref()))
}

/// Build a `$list` `$value` of `$str` elements (e.g. `read-dir`'s entry names).
fn build_str_list(store: &mut impl AsContextMut, gc: &GcTypes, items: &[String]) -> Val {
	let strs: Vec<Val> = items
		.iter()
		.map(|s| build_strlike(&mut *store, gc, TAG_STR, s.as_bytes()))
		.collect();
	let arr_pre = ArrayRefPre::new(&mut *store, gc.valarray.clone());
	let arr = ArrayRef::new_fixed(&mut *store, &arr_pre, &strs).expect("build $valarray");
	let pre = StructRefPre::new(&mut *store, gc.list.clone());
	// $list is { tag, elems, length } — length == capacity here (no spare).
	let s = StructRef::new(
		&mut *store,
		&pre,
		&[
			Val::I32(TAG_LIST),
			Val::AnyRef(Some(arr.into())),
			Val::I32(items.len() as i32),
		],
	)
	.expect("build $list");
	Val::AnyRef(Some(s.to_anyref()))
}

/// Capture the module's GC types from the witness on the first io call, caching
/// them in `HostState` so subsequent calls skip the reflection.
fn ensure_types(caller: &mut Caller<HostState>, witness: &Val) -> GcTypes {
	if let Some(t) = caller.data().gc_types.clone() {
		return t;
	}
	let t = capture_gc_types(caller, witness);
	caller.data_mut().gc_types = Some(t.clone());
	t
}

/// The cached GC types (set by the most recent io op's witness) — for `io-last-error`,
/// which carries no witness because it always follows a failing op that set them.
fn cached_types(caller: &Caller<HostState>) -> GcTypes {
	caller
		.data()
		.gc_types
		.clone()
		.expect("io-last-error called before any io op cached the GC types")
}

/// Extract a `$str` argument as a Rust `String` (UTF-8 lossy, like the VM).
fn arg_string(store: &mut impl AsContextMut, v: &Val) -> String {
	String::from_utf8_lossy(&raw_value_bytes(store, v)).into_owned()
}

/// Stash an io error message for the next `io-last-error` query.
fn set_io_err(caller: &mut Caller<HostState>, msg: String) {
	caller.data_mut().last_error = msg;
}

pub fn engine() -> Engine {
	let mut config = Config::new();
	config.wasm_reference_types(true);
	config.wasm_function_references(true);
	config.wasm_gc(true);
	config.wasm_tail_call(true);
	// Use the null collector (allocate, never collect): these fixtures are tiny,
	// short-lived programs, so never collecting is the fastest option. (This also
	// used to dodge a wasmtime 30 deferred-reference-counting collector panic
	// ("invalid VMGcKind"); that bug is fixed as of wasmtime 45, so the drc
	// collector works too — null just stays cheaper for this workload.)
	config.collector(wasmtime::Collector::Null);
	Engine::new(&config).expect("engine")
}

/// Format a boxed `$value` exactly as `vm::Value`'s `Display` would, for the
/// `print` host import.
fn format_value(store: &mut impl AsContextMut, val: &Val) -> String {
	let any = match val {
		Val::AnyRef(Some(r)) => *r,
		Val::AnyRef(None) => return "()".to_string(),
		other => return format!("<non-ref {other:?}>"),
	};
	format_anyref(store, any)
}

fn format_anyref(store: &mut impl AsContextMut, any: Rooted<AnyRef>) -> String {
	let s = any
		.as_struct(&mut *store)
		.expect("as_struct")
		.expect("a $value struct");
	let tag = match s.field(&mut *store, 0).expect("tag field") {
		Val::I32(t) => t,
		other => panic!("tag not i32: {other:?}"),
	};
	match tag {
		TAG_NOTHING => "()".to_string(),
		TAG_BOOL => match s.field(&mut *store, 1).expect("bool field") {
			Val::I32(b) => (b != 0).to_string(),
			o => panic!("bool payload: {o:?}"),
		},
		TAG_INT => match s.field(&mut *store, 1).expect("int field") {
			Val::I64(n) => n.to_string(),
			o => panic!("int payload: {o:?}"),
		},
		TAG_FLOAT => match s.field(&mut *store, 1).expect("float field") {
			Val::F64(bits) => {
				let n = f64::from_bits(bits);
				if n.fract() == 0.0 && n.is_finite() {
					format!("{n:.1}")
				} else {
					format!("{n}")
				}
			}
			o => panic!("float payload: {o:?}"),
		},
		TAG_STR => {
			let arr = match s.field(&mut *store, 1).expect("str field") {
				Val::AnyRef(Some(r)) => r
					.as_array(&mut *store)
					.expect("as_array")
					.expect("bytes array"),
				o => panic!("str payload: {o:?}"),
			};
			let len = arr.len(&mut *store).expect("array len");
			let mut bytes = Vec::with_capacity(len as usize);
			for i in 0..len {
				match arr.get(&mut *store, i).expect("array get") {
					Val::I32(b) => bytes.push(b as u8),
					o => panic!("byte elem: {o:?}"),
				}
			}
			String::from_utf8_lossy(&bytes).into_owned()
		}
		TAG_BYTES => {
			// Same `$bytes` backing as a string, rendered in the single-quote
			// literal form (`'..\xNN'`) that `vm::Value`'s Display uses.
			let arr = match s.field(&mut *store, 1).expect("bytes field") {
				Val::AnyRef(Some(r)) => r
					.as_array(&mut *store)
					.expect("as_array")
					.expect("bytes array"),
				o => panic!("bytes payload: {o:?}"),
			};
			let len = arr.len(&mut *store).expect("array len");
			let mut out = String::from("'");
			for i in 0..len {
				let byte = match arr.get(&mut *store, i).expect("array get") {
					Val::I32(b) => b as u8,
					o => panic!("byte elem: {o:?}"),
				};
				match byte {
					b'\\' => out.push_str("\\\\"),
					b'\'' => out.push_str("\\'"),
					0x20..=0x7e => out.push(byte as char),
					_ => out.push_str(&format!("\\x{byte:02x}")),
				}
			}
			out.push('\'');
			out
		}
		TAG_DURATION => {
			// A `duration` reuses the `$int` shape; format its i64 nanos canonically
			// (descending d/h/m/s/ms/us/ns segments), mirroring `vm::Value`'s Display.
			let nanos = match s.field(&mut *store, 1).expect("duration field") {
				Val::I64(n) => n,
				o => panic!("duration payload: {o:?}"),
			};
			format_duration(nanos)
		}
		TAG_VARIANT => {
			// `name` (field 2, a $str) then each payload element, space-separated:
			// e.g. `color.red`, `shape.square 5`.
			let nf = s.field(&mut *store, 2).expect("variant name");
			let pf = s.field(&mut *store, 3).expect("variant payload");
			let name = format_value(store, &nf);
			let payload = format_elems(store, &pf);
			let mut out = name;
			for p in payload {
				out.push(' ');
				out.push_str(&p);
			}
			out
		}
		TAG_TUPLE => {
			let f = s.field(&mut *store, 1).expect("tuple elems");
			format!("({})", format_elems(store, &f).join(", "))
		}
		TAG_LIST => {
			// $list is { tag, elems, length } — only the first `length` elements are
			// live (the backing array may have spare capacity after `list.push`).
			let f = s.field(&mut *store, 1).expect("list elems");
			let len = match s.field(&mut *store, 2).expect("list length") {
				Val::I32(n) => n as usize,
				o => panic!("list length: {o:?}"),
			};
			let mut elems = format_elems(store, &f);
			elems.truncate(len);
			format!("[{}]", elems.join(", "))
		}
		TAG_RECORD => {
			let nf = s.field(&mut *store, 1).expect("record names");
			let vf = s.field(&mut *store, 2).expect("record values");
			let names = format_elems(store, &nf);
			let values = format_elems(store, &vf);
			let pairs: Vec<String> = names
				.iter()
				.zip(&values)
				.map(|(k, v)| format!("{k}: {v}"))
				.collect();
			format!("{{{}}}", pairs.join(", "))
		}
		TAG_REF => {
			// `ref <inner>` — the cell's value, recursively (matches `vm::Value`).
			let cell = s.field(&mut *store, 1).expect("ref cell");
			format!("ref {}", format_value(store, &cell))
		}
		TAG_DICT => {
			// `{k: v, ...}` — entries are `$tuple` (key, value), insertion order.
			let ef = s.field(&mut *store, 1).expect("dict entries");
			let entries = format_elems(store, &ef);
			// each element formats as a tuple "(k, v)"; reshape to "k: v"
			let pairs: Vec<String> = entries
				.iter()
				.map(|e| {
					let inner = e
						.strip_prefix('(')
						.and_then(|s| s.strip_suffix(')'))
						.unwrap_or(e);
					match inner.split_once(", ") {
						Some((k, v)) => format!("{k}: {v}"),
						None => inner.to_string(),
					}
				})
				.collect();
			format!("{{{}}}", pairs.join(", "))
		}
		other => format!("<tag {other}>"),
	}
}

/// The raw bytes backing a `$str`/`$bytes` value (field 1 of the struct),
/// without any Display formatting — for the `io.write-bytes` raw writers.
fn raw_value_bytes(store: &mut impl AsContextMut, val: &Val) -> Vec<u8> {
	let Val::AnyRef(Some(r)) = val else {
		return Vec::new();
	};
	let s = r
		.as_struct(&mut *store)
		.expect("as_struct")
		.expect("a $value");
	let arr = match s.field(&mut *store, 1).expect("bytes field") {
		Val::AnyRef(Some(a)) => a
			.as_array(&mut *store)
			.expect("as_array")
			.expect("bytes array"),
		o => panic!("bytes payload: {o:?}"),
	};
	let len = arr.len(&mut *store).expect("array len");
	let mut bytes = Vec::with_capacity(len as usize);
	for i in 0..len {
		match arr.get(&mut *store, i).expect("array get") {
			Val::I32(b) => bytes.push(b as u8),
			o => panic!("byte elem: {o:?}"),
		}
	}
	bytes
}

/// Format each element of a `$valarray` field value.
fn format_elems(store: &mut impl AsContextMut, arr: &Val) -> Vec<String> {
	let array = match arr {
		Val::AnyRef(Some(r)) => r
			.as_array(&mut *store)
			.expect("as_array")
			.expect("valarray"),
		o => panic!("expected array: {o:?}"),
	};
	let len = array.len(&mut *store).expect("array len");
	let mut out = Vec::with_capacity(len as usize);
	for i in 0..len {
		let elem = array.get(&mut *store, i).expect("array get");
		out.push(format_value(store, &elem));
	}
	out
}

/// Canonical duration rendering — the i64 nanos broken into descending
/// d/h/m/s/ms/us/ns segments. Mirrors `vm::value::format_duration`.
fn format_duration(nanos: i64) -> String {
	if nanos == 0 {
		return "0s".to_string();
	}
	let (sign, mut rem): (&str, u128) = if nanos < 0 {
		("-", (nanos as i128).unsigned_abs())
	} else {
		("", nanos as u128)
	};
	const UNITS: [(u128, &str); 7] = [
		(86_400_000_000_000, "d"),
		(3_600_000_000_000, "h"),
		(60_000_000_000, "m"),
		(1_000_000_000, "s"),
		(1_000_000, "ms"),
		(1_000, "us"),
		(1, "ns"),
	];
	let mut out = String::from(sign);
	for (per, name) in UNITS {
		if rem >= per {
			out.push_str(&(rem / per).to_string());
			out.push_str(name);
			rem %= per;
		}
	}
	out
}

// --------------------------------------------------------------------------
// `core.net` — the host-side socket table + I/O reactor. The WasmGC analogue of
// `vm::net`: the same byte-level TCP ops plus a `polling` readiness reactor. The
// in-wasm scheduler owns the loop; when its ready queue empties and socket I/O is
// in flight, it calls the blocking `net-poll` import here (mirroring the VM's
// `block_until_ready` reactor step). The suspending ops (accept/read/write) are
// *non-blocking* host calls: on `WouldBlock` they register the socket's fd under
// the parked fiber's id (token = fid) and signal would-block; the scheduler parks
// the fiber and later drives `net-poll`. listen/close/local-addr/connect are
// synchronous (v1 connect blocks — a loopback dial completes in-kernel).
// --------------------------------------------------------------------------

/// A live socket the program holds a handle to (an opaque `int` id into `sockets`).
enum SocketEntry {
	Listener(TcpListener),
	Conn(TcpStream),
}

impl SocketEntry {
	fn raw_fd(&self) -> RawFd {
		match self {
			SocketEntry::Listener(l) => l.as_raw_fd(),
			SocketEntry::Conn(c) => c.as_raw_fd(),
		}
	}
}

/// The outcome of one host net op, before it's shaped into a `result` `$value`.
/// `OkInt` rides the i32 `n` return channel (boxed in wasm); the value-bearing
/// arms build a primitive `$value` payload; `WouldBlock` signals a park.
enum NetRet {
	OkInt(i32), // a listener/connection id, or a bytes-written count
	OkBytes(Vec<u8>),
	OkStr(String),
	OkNothing,
	Err(String),
	WouldBlock,
}

/// Read- vs write-readiness for a park (mirrors `vm::net::Interest`).
#[derive(Clone, Copy)]
enum Interest {
	Read,
	Write,
}

/// All `core.net` runtime state: the socket table plus the readiness reactor.
/// Lives in `HostState` so it persists across host calls for the whole run.
struct HostNet {
	sockets: HashMap<u32, SocketEntry>,
	next_id: u32,
	/// Created lazily on the first park — a net-free program never makes one.
	poller: Option<Poller>,
	events: Events,
	/// Parked fibers keyed by id (token = fid) → the socket fd to deregister on wake.
	waits: HashMap<i32, RawFd>,
	/// Fibers whose socket is ready, buffered across `net-poll` calls (one `wait`
	/// can surface several; the scheduler consumes one fid per poll).
	ready: VecDeque<i32>,
}

impl Default for HostNet {
	fn default() -> Self {
		HostNet {
			sockets: HashMap::new(),
			next_id: 0,
			poller: None,
			events: Events::new(),
			waits: HashMap::new(),
			ready: VecDeque::new(),
		}
	}
}

impl HostNet {
	fn store(&mut self, e: SocketEntry) -> u32 {
		let id = self.next_id;
		self.next_id += 1;
		self.sockets.insert(id, e);
		id
	}

	fn listen(&mut self, addr: &str) -> NetRet {
		match TcpListener::bind(addr) {
			Ok(l) => match l.set_nonblocking(true) {
				Ok(()) => NetRet::OkInt(self.store(SocketEntry::Listener(l)) as i32),
				Err(e) => NetRet::Err(e.to_string()),
			},
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	fn close(&mut self, id: u32) -> NetRet {
		match self.sockets.remove(&id) {
			Some(_) => NetRet::OkNothing,
			None => NetRet::Err(format!("net.close: no such socket ({id})")),
		}
	}

	fn local_addr(&self, id: u32) -> NetRet {
		let addr = match self.sockets.get(&id) {
			Some(SocketEntry::Listener(l)) => l.local_addr(),
			Some(SocketEntry::Conn(c)) => c.local_addr(),
			None => return NetRet::Err(format!("net.local-addr: no such socket ({id})")),
		};
		match addr {
			Ok(a) => NetRet::OkStr(a.to_string()),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	fn connect(&mut self, addr: &str) -> NetRet {
		match TcpStream::connect(addr) {
			Ok(s) => match s.set_nonblocking(true) {
				Ok(()) => NetRet::OkInt(self.store(SocketEntry::Conn(s)) as i32),
				Err(e) => NetRet::Err(e.to_string()),
			},
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	fn try_accept(&mut self, fid: i32, lid: u32) -> NetRet {
		let res = match self.sockets.get(&lid) {
			Some(SocketEntry::Listener(l)) => l.accept(),
			_ => return NetRet::Err(format!("net.accept: not a listener ({lid})")),
		};
		match res {
			Ok((stream, _peer)) => match stream.set_nonblocking(true) {
				Ok(()) => NetRet::OkInt(self.store(SocketEntry::Conn(stream)) as i32),
				Err(e) => NetRet::Err(e.to_string()),
			},
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => self.park(fid, lid, Interest::Read),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	fn try_read(&mut self, fid: i32, cid: u32, max: usize) -> NetRet {
		let mut buf = vec![0u8; max];
		let res = match self.sockets.get_mut(&cid) {
			Some(SocketEntry::Conn(c)) => c.read(&mut buf),
			_ => return NetRet::Err(format!("net.read: not a connection ({cid})")),
		};
		match res {
			// n == 0 is a clean EOF: an empty `bytes`, distinguishable by length.
			Ok(n) => {
				buf.truncate(n);
				NetRet::OkBytes(buf)
			}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => self.park(fid, cid, Interest::Read),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	fn try_write(&mut self, fid: i32, cid: u32, data: &[u8]) -> NetRet {
		let res = match self.sockets.get_mut(&cid) {
			Some(SocketEntry::Conn(c)) => c.write(data),
			_ => return NetRet::Err(format!("net.write: not a connection ({cid})")),
		};
		match res {
			Ok(n) => NetRet::OkInt(n as i32),
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => self.park(fid, cid, Interest::Write),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	/// Register fiber `fid` against socket `sid`'s readiness (token = fid), then
	/// report would-block. Mirrors `vm::net::reactor_park`.
	fn park(&mut self, fid: i32, sid: u32, interest: Interest) -> NetRet {
		let fd = match self.sockets.get(&sid) {
			Some(e) => e.raw_fd(),
			None => return NetRet::Err(format!("net: park on unknown socket {sid}")),
		};
		if self.poller.is_none() {
			match Poller::new() {
				Ok(p) => self.poller = Some(p),
				Err(e) => return NetRet::Err(format!("net: poller: {e}")),
			}
		}
		let ev = match interest {
			Interest::Read => Event::readable(fid as usize),
			Interest::Write => Event::writable(fid as usize),
		};
		// SAFETY: the socket lives in `sockets` and is removed from the poller
		// (`delete`) on wake or unwatch before it can be closed. One fiber owns a
		// socket op at a time, so an fd is never double-added.
		if let Err(e) = unsafe { self.poller.as_ref().unwrap().add(fd, ev) } {
			return NetRet::Err(format!("net: poller add: {e}"));
		}
		self.waits.insert(fid, fd);
		NetRet::WouldBlock
	}

	/// Block until a parked socket is ready (or `deadline` nanos elapse; `-1` =
	/// block indefinitely), returning one woken fid (`-1` on timeout / nothing
	/// pending). Extra simultaneously-ready fids are buffered for later calls.
	/// Mirrors `vm::net::reactor_poll` + the scheduler's per-fiber consumption.
	fn poll(&mut self, deadline: i64) -> i32 {
		if self.ready.is_empty() {
			if self.waits.is_empty() {
				return -1;
			}
			let timeout = if deadline < 0 {
				None
			} else {
				Some(Duration::from_nanos(deadline as u64))
			};
			let HostNet {
				poller,
				events,
				waits,
				ready,
				..
			} = self;
			let poller = poller.as_mut().expect("poller exists when waits non-empty");
			events.clear();
			if poller.wait(events, timeout).is_err() {
				return -1;
			}
			for ev in events.iter() {
				let fid = ev.key as i32;
				if let Some(fd) = waits.remove(&fid) {
					// SAFETY: same fd we added; deleted before the socket is dropped.
					let _ = poller.delete(unsafe { BorrowedFd::borrow_raw(fd) });
					ready.push_back(fid);
				}
			}
		}
		self.ready.pop_front().unwrap_or(-1)
	}

	/// Drop a parked I/O wait (on cancellation / reaping). Idempotent. Mirrors
	/// `vm::net::reactor_deregister`.
	fn unwatch(&mut self, fid: i32) {
		if let Some(fd) = self.waits.remove(&fid) {
			if let Some(p) = &self.poller {
				// SAFETY: same fd we added; deleted before the socket is dropped.
				let _ = p.delete(unsafe { BorrowedFd::borrow_raw(fd) });
			}
		}
	}
}

/// Read a `$int`(-shaped) `$value` argument as an i64 (its field-1 payload).
fn arg_int(store: &mut impl AsContextMut, v: &Val) -> i64 {
	let Val::AnyRef(Some(r)) = v else {
		return 0;
	};
	let s = r
		.as_struct(&mut *store)
		.expect("as_struct")
		.expect("a $value");
	match s.field(&mut *store, 1).expect("int field") {
		Val::I64(n) => n,
		o => panic!("int payload: {o:?}"),
	}
}

/// Shape a `NetRet` into the `(status:i32, n:i32, payload:ref null $value)` triple
/// every net host import returns. status 0 = ok / 1 = would-block / 2 = err. The
/// wasm side wraps it: status 0 boxes `n` (OkInt ops) or wraps `payload` in `ok`;
/// status 2 wraps `payload` in `err`; status 1 parks the fiber.
fn set_net_results(store: &mut impl AsContextMut, gc: &GcTypes, ret: NetRet, results: &mut [Val]) {
	let (status, n, payload): (i32, i32, Val) = match ret {
		NetRet::OkInt(v) => (0, v, Val::AnyRef(None)),
		NetRet::OkBytes(b) => (0, 0, build_strlike(store, gc, TAG_BYTES, &b)),
		NetRet::OkStr(s) => (0, 0, build_strlike(store, gc, TAG_STR, s.as_bytes())),
		NetRet::OkNothing => (0, 0, build_nothing(store, gc)),
		NetRet::Err(e) => (2, 0, build_strlike(store, gc, TAG_STR, e.as_bytes())),
		NetRet::WouldBlock => (1, 0, Val::AnyRef(None)),
	};
	results[0] = Val::I32(status);
	results[1] = Val::I32(n);
	results[2] = payload;
}

fn instantiate_module(
	engine: &Engine,
	module: &Module,
	io: Box<dyn HostIo>,
) -> Result<(Store<HostState>, Instance), String> {
	let mut store = Store::new(
		engine,
		HostState {
			io,
			fail: None,
			last_error: String::new(),
			gc_types: None,
			net: HostNet::default(),
		},
	);
	let mut linker: Linker<HostState> = Linker::new(engine);
	// print : (ref null $value) -> ()  — host accepts the broader `anyref`.
	let print_ty = FuncType::new(engine, [ValType::ANYREF], []);
	linker
		.func_new("pluma", "print", print_ty, |mut caller, args, _results| {
			// Bound the temporary GC roots created while reflecting the value: a
			// `RootScope` frees them on drop, so repeated prints don't accumulate
			// roots (which otherwise corrupts wasmtime's GC under collection).
			let mut line = {
				let mut scope = RootScope::new(&mut caller);
				format_value(&mut scope, &args[0]).into_bytes()
			};
			line.push(b'\n');
			caller.data_mut().io.write_out(&line);
			Ok(())
		})
		.expect("define print");
	// The `core.io` writers. `print`/`print-err` append a newline; `write`/
	// `write-err` don't. The `*-err` pair targets stderr. `*-bytes` write a `bytes`
	// value's raw bytes (no Display formatting).
	let io_ty = FuncType::new(engine, [ValType::ANYREF], []);
	for (name, to_err, newline, raw) in [
		("io-print", false, true, false),
		("io-write", false, false, false),
		("io-print-err", true, true, false),
		("io-write-err", true, false, false),
		("io-write-bytes", false, false, true),
		("io-write-err-bytes", true, false, true),
	] {
		linker
			.func_new(
				"pluma",
				name,
				io_ty.clone(),
				move |mut caller, args, _results| {
					let mut bytes = {
						let mut scope = RootScope::new(&mut caller);
						if raw {
							raw_value_bytes(&mut scope, &args[0])
						} else {
							format_value(&mut scope, &args[0]).into_bytes()
						}
					};
					if newline {
						bytes.push(b'\n');
					}
					if to_err {
						caller.data_mut().io.write_err(&bytes);
					} else {
						caller.data_mut().io.write_out(&bytes);
					}
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}
	// io.fail msg : stash the message, then trap. The runner reads the message back
	// to form the `runtime error: <msg>` status (mirrors the VM's abort).
	linker
		.func_new(
			"pluma",
			"io-fail",
			io_ty.clone(),
			|mut caller, args, _results| {
				let msg = {
					let mut scope = RootScope::new(&mut caller);
					format_value(&mut scope, &args[0])
				};
				caller.data_mut().fail = Some(msg);
				Err(wasmtime::Error::msg("io.fail"))
			},
		)
		.expect("define io-fail");
	// float_to_str : (f64, $bytes buf) -> i32 len. Format the float as `vm::Value`'s
	// Display does, write the bytes into the caller-provided GC byte array, return
	// the length. (A real browser target would delegate to JS similarly.)
	let f2s_ty = FuncType::new(engine, [ValType::F64, ValType::ANYREF], [ValType::I32]);
	linker
		.func_new(
			"pluma",
			"float_to_str",
			f2s_ty,
			|mut caller, args, results| {
				let n = match args[0] {
					Val::F64(bits) => f64::from_bits(bits),
					ref o => panic!("float_to_str arg: {o:?}"),
				};
				let s = if n.fract() == 0.0 && n.is_finite() {
					format!("{n:.1}")
				} else {
					format!("{n}")
				};
				let buf = match &args[1] {
					Val::AnyRef(Some(r)) => r.as_array(&mut caller).expect("array").expect("buf"),
					o => panic!("float_to_str buf: {o:?}"),
				};
				let bytes = s.as_bytes();
				for (i, &byte) in bytes.iter().enumerate() {
					buf
						.set(&mut caller, i as u32, Val::I32(byte as i32))
						.expect("buf set");
				}
				results[0] = Val::I32(bytes.len() as i32);
				Ok(())
			},
		)
		.expect("define float_to_str");
	// Unary float math host imports: raw `(f64) -> f64`, the same libm calls the
	// VM makes (`f64::ln`/`log10`/`log2`/`exp`/`sin`/`cos`). A browser target would
	// import `Math.log`/`Math.log10`/… here instead.
	let f64_unary_ty = FuncType::new(engine, [ValType::F64], [ValType::F64]);
	for (name, f) in [
		("math-log", f64::ln as fn(f64) -> f64),
		("math-log10", f64::log10),
		("math-log2", f64::log2),
		("math-exp", f64::exp),
		("math-sin", f64::sin),
		("math-cos", f64::cos),
	] {
		linker
			.func_new(
				"pluma",
				name,
				f64_unary_ty.clone(),
				move |_caller, args, results| {
					let x = match args[0] {
						Val::F64(bits) => f64::from_bits(bits),
						ref o => panic!("{name} arg: {o:?}"),
					};
					results[0] = Val::F64(f(x).to_bits());
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// `core.io` host imports (the server-platform capability). Each takes a trailing
	// type witness (see `emit_io_witness`) the host reflects to build its `$value`
	// return. The reads/writes return a primitive `$value`-or-null that wasm wraps in
	// `ok`/`err` via `__io_result`; `file-exists`/`is-dir` return a bare `$bool`. All
	// fs ops use real `std::fs` so the `err` strings match the VM's errno text.
	// The io imports return a concrete `(ref null $value)`, not the broader `anyref`
	// (a result must be a *subtype* of what the module imports). Recover that exact
	// type from the `_entry` export's signature (its env param / return is `$value`).
	let value_ty: ValType = module
		.exports()
		.find(|e| e.name() == "_entry")
		.and_then(|e| match e.ty() {
			ExternType::Func(f) => f.results().next(),
			_ => None,
		})
		.expect("_entry export with a $value result type");
	let io2 = FuncType::new(
		engine,
		[ValType::ANYREF, ValType::ANYREF],
		[value_ty.clone()],
	);
	let io3 = FuncType::new(
		engine,
		[ValType::ANYREF, ValType::ANYREF, ValType::ANYREF],
		[value_ty.clone()],
	);
	let io0 = FuncType::new(engine, [], [value_ty.clone()]);

	// read-file / read-file-bytes: path is args[0], witness args[1].
	for (name, as_bytes) in [("io-read-file", false), ("io-read-file-bytes", true)] {
		linker
			.func_new(
				"pluma",
				name,
				io2.clone(),
				move |mut caller, args, results| {
					let gc = ensure_types(&mut caller, &args[1]);
					let path = arg_string(&mut caller, &args[0]);
					results[0] = if as_bytes {
						match std::fs::read(&path) {
							Ok(b) => build_strlike(&mut caller, &gc, TAG_BYTES, &b),
							Err(e) => {
								set_io_err(&mut caller, e.to_string());
								Val::AnyRef(None)
							}
						}
					} else {
						match std::fs::read_to_string(&path) {
							Ok(s) => build_strlike(&mut caller, &gc, TAG_STR, s.as_bytes()),
							Err(e) => {
								set_io_err(&mut caller, e.to_string());
								Val::AnyRef(None)
							}
						}
					};
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// write-file / append-file (+ bytes variants): path args[0], data args[1], witness
	// args[2]. Return `nothing` on success.
	for (name, append, as_bytes) in [
		("io-write-file", false, false),
		("io-append-file", true, false),
		("io-write-file-bytes", false, true),
		("io-append-file-bytes", true, true),
	] {
		linker
			.func_new(
				"pluma",
				name,
				io3.clone(),
				move |mut caller, args, results| {
					let gc = ensure_types(&mut caller, &args[2]);
					let path = arg_string(&mut caller, &args[0]);
					let data = if as_bytes {
						raw_value_bytes(&mut caller, &args[1])
					} else {
						arg_string(&mut caller, &args[1]).into_bytes()
					};
					let res = if append {
						use std::io::Write;
						std::fs::OpenOptions::new()
							.create(true)
							.append(true)
							.open(&path)
							.and_then(|mut f| f.write_all(&data))
					} else {
						std::fs::write(&path, &data)
					};
					results[0] = match res {
						Ok(()) => build_nothing(&mut caller, &gc),
						Err(e) => {
							set_io_err(&mut caller, e.to_string());
							Val::AnyRef(None)
						}
					};
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// delete-file / make-dir (mkdir -p): path args[0], witness args[1].
	for (name, is_mkdir) in [("io-delete-file", false), ("io-make-dir", true)] {
		linker
			.func_new(
				"pluma",
				name,
				io2.clone(),
				move |mut caller, args, results| {
					let gc = ensure_types(&mut caller, &args[1]);
					let path = arg_string(&mut caller, &args[0]);
					let res = if is_mkdir {
						std::fs::create_dir_all(&path)
					} else {
						std::fs::remove_file(&path)
					};
					results[0] = match res {
						Ok(()) => build_nothing(&mut caller, &gc),
						Err(e) => {
							set_io_err(&mut caller, e.to_string());
							Val::AnyRef(None)
						}
					};
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// file-exists / is-dir: path args[0], witness args[1]. Return a bare `$bool`.
	for (name, is_dir) in [("io-file-exists", false), ("io-is-dir", true)] {
		linker
			.func_new(
				"pluma",
				name,
				io2.clone(),
				move |mut caller, args, results| {
					let gc = ensure_types(&mut caller, &args[1]);
					let path = arg_string(&mut caller, &args[0]);
					let p = std::path::Path::new(&path);
					let b = if is_dir { p.is_dir() } else { p.exists() };
					results[0] = build_bool(&mut caller, &gc, b);
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// read-dir: path args[0], witness args[1]. Entry names only, sorted (VM parity).
	linker
		.func_new(
			"pluma",
			"io-read-dir",
			io2.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let path = arg_string(&mut caller, &args[0]);
				results[0] = match std::fs::read_dir(&path) {
					Ok(entries) => {
						let mut names: Vec<String> = Vec::new();
						let mut read_err: Option<String> = None;
						for entry in entries {
							match entry {
								Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
								Err(e) => {
									read_err = Some(e.to_string());
									break;
								}
							}
						}
						match read_err {
							Some(msg) => {
								set_io_err(&mut caller, msg);
								Val::AnyRef(None)
							}
							None => {
								names.sort();
								build_str_list(&mut caller, &gc, &names)
							}
						}
					}
					Err(e) => {
						set_io_err(&mut caller, e.to_string());
						Val::AnyRef(None)
					}
				};
				Ok(())
			},
		)
		.expect("define io-read-dir");

	// read / read-all / read-all-bytes: unit args[0], witness args[1].
	linker
		.func_new(
			"pluma",
			"io-read",
			io2.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let line = caller.data_mut().io.read_line();
				results[0] = match line {
					Some(line) => build_strlike(&mut caller, &gc, TAG_STR, line.as_bytes()),
					None => {
						set_io_err(&mut caller, "EOF".to_string());
						Val::AnyRef(None)
					}
				};
				Ok(())
			},
		)
		.expect("define io-read");
	linker
		.func_new(
			"pluma",
			"io-read-all",
			io2.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let bytes = caller.data_mut().io.read_rest();
				let s = String::from_utf8_lossy(&bytes).into_owned();
				results[0] = build_strlike(&mut caller, &gc, TAG_STR, s.as_bytes());
				Ok(())
			},
		)
		.expect("define io-read-all");
	linker
		.func_new(
			"pluma",
			"io-read-all-bytes",
			io2.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let bytes = caller.data_mut().io.read_rest();
				results[0] = build_strlike(&mut caller, &gc, TAG_BYTES, &bytes);
				Ok(())
			},
		)
		.expect("define io-read-all-bytes");

	// io-last-error: the message the last failed io call stashed, as a `$str`. No
	// witness — it rides the GC types the failing op already cached.
	linker
		.func_new(
			"pluma",
			"io-last-error",
			io0,
			|mut caller, _args, results| {
				let gc = cached_types(&caller);
				let msg = caller.data().last_error.clone();
				results[0] = build_strlike(&mut caller, &gc, TAG_STR, msg.as_bytes());
				Ok(())
			},
		)
		.expect("define io-last-error");

	// `core.net` host imports (see `HostNet`). Each fallible op returns the
	// `(status:i32, n:i32, payload:ref null $value)` triple `set_net_results` builds
	// — ints ride `n` (boxed in wasm), values/errs ride `payload`. The synchronous
	// ops (listen/close/local-addr/connect) take a trailing type witness (like the
	// io imports) so the host can build their `$value` payloads; the suspending ops
	// (accept/read/write) take the parked fiber's id first and ride the GC types the
	// preceding listen/connect already cached. `net-poll`/`net-unwatch` drive the
	// reactor from the in-wasm scheduler's block step.
	let net3 = FuncType::new(
		engine,
		[ValType::ANYREF, ValType::ANYREF],
		[ValType::I32, ValType::I32, value_ty.clone()],
	);
	let net_accept_ty = FuncType::new(
		engine,
		[ValType::I32, ValType::ANYREF],
		[ValType::I32, ValType::I32, value_ty.clone()],
	);
	let net_rw_ty = FuncType::new(
		engine,
		[ValType::I32, ValType::ANYREF, ValType::ANYREF],
		[ValType::I32, ValType::I32, value_ty.clone()],
	);
	// net-listen addr witness -> (status, listener-id, _).
	linker
		.func_new(
			"pluma",
			"net-listen",
			net3.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let addr = arg_string(&mut caller, &args[0]);
				let ret = caller.data_mut().net.listen(&addr);
				set_net_results(&mut caller, &gc, ret, results);
				Ok(())
			},
		)
		.expect("define net-listen");
	// net-close conn witness -> (status, _, nothing/err).
	linker
		.func_new(
			"pluma",
			"net-close",
			net3.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let id = arg_int(&mut caller, &args[0]) as u32;
				let ret = caller.data_mut().net.close(id);
				set_net_results(&mut caller, &gc, ret, results);
				Ok(())
			},
		)
		.expect("define net-close");
	// net-local-addr listener witness -> (status, _, addr/err).
	linker
		.func_new(
			"pluma",
			"net-local-addr",
			net3.clone(),
			|mut caller, args, results| {
				let gc = ensure_types(&mut caller, &args[1]);
				let id = arg_int(&mut caller, &args[0]) as u32;
				let ret = caller.data_mut().net.local_addr(id);
				set_net_results(&mut caller, &gc, ret, results);
				Ok(())
			},
		)
		.expect("define net-local-addr");
	// net-connect addr witness -> (status, connection-id, err). v1 blocks.
	linker
		.func_new("pluma", "net-connect", net3, |mut caller, args, results| {
			let gc = ensure_types(&mut caller, &args[1]);
			let addr = arg_string(&mut caller, &args[0]);
			let ret = caller.data_mut().net.connect(&addr);
			set_net_results(&mut caller, &gc, ret, results);
			Ok(())
		})
		.expect("define net-connect");
	// net-accept fid listener -> (status, connection-id, err) | would-block.
	linker
		.func_new(
			"pluma",
			"net-accept",
			net_accept_ty,
			|mut caller, args, results| {
				let gc = cached_types(&caller);
				let fid = match args[0] {
					Val::I32(f) => f,
					ref o => panic!("net-accept fid: {o:?}"),
				};
				let lid = arg_int(&mut caller, &args[1]) as u32;
				let ret = caller.data_mut().net.try_accept(fid, lid);
				set_net_results(&mut caller, &gc, ret, results);
				Ok(())
			},
		)
		.expect("define net-accept");
	// net-read fid conn max -> (status, _, bytes/err) | would-block.
	linker
		.func_new(
			"pluma",
			"net-read",
			net_rw_ty.clone(),
			|mut caller, args, results| {
				let gc = cached_types(&caller);
				let fid = match args[0] {
					Val::I32(f) => f,
					ref o => panic!("net-read fid: {o:?}"),
				};
				let cid = arg_int(&mut caller, &args[1]) as u32;
				let max = arg_int(&mut caller, &args[2]).max(0) as usize;
				let ret = caller.data_mut().net.try_read(fid, cid, max);
				set_net_results(&mut caller, &gc, ret, results);
				Ok(())
			},
		)
		.expect("define net-read");
	// net-write fid conn data -> (status, bytes-written, err) | would-block.
	linker
		.func_new(
			"pluma",
			"net-write",
			net_rw_ty,
			|mut caller, args, results| {
				let gc = cached_types(&caller);
				let fid = match args[0] {
					Val::I32(f) => f,
					ref o => panic!("net-write fid: {o:?}"),
				};
				let cid = arg_int(&mut caller, &args[1]) as u32;
				let data = raw_value_bytes(&mut caller, &args[2]);
				let ret = caller.data_mut().net.try_write(fid, cid, &data);
				set_net_results(&mut caller, &gc, ret, results);
				Ok(())
			},
		)
		.expect("define net-write");
	// net-poll deadline-nanos -> woken fid (-1 = timeout / nothing pending).
	let net_poll_ty = FuncType::new(engine, [ValType::I64], [ValType::I32]);
	linker
		.func_new(
			"pluma",
			"net-poll",
			net_poll_ty,
			|mut caller, args, results| {
				let deadline = match args[0] {
					Val::I64(d) => d,
					ref o => panic!("net-poll deadline: {o:?}"),
				};
				results[0] = Val::I32(caller.data_mut().net.poll(deadline));
				Ok(())
			},
		)
		.expect("define net-poll");
	// net-unwatch fid -> () : drop a cancelled fiber's reactor registration.
	let net_unwatch_ty = FuncType::new(engine, [ValType::I32], []);
	linker
		.func_new(
			"pluma",
			"net-unwatch",
			net_unwatch_ty,
			|mut caller, args, _results| {
				let fid = match args[0] {
					Val::I32(f) => f,
					ref o => panic!("net-unwatch fid: {o:?}"),
				};
				caller.data_mut().net.unwatch(fid);
				Ok(())
			},
		)
		.expect("define net-unwatch");

	let instance = linker
		.instantiate(&mut store, module)
		.map_err(|e| format!("instantiate error: {e}"))?;
	Ok((store, instance))
}

/// Compile + instantiate `bytes` and run `_entry` once with stdin from a slice,
/// capturing stdout (the conformance diff path).
pub fn run_wasm(bytes: &[u8], stdin: &[u8]) -> RunResult {
	let engine = engine();
	let module = match Module::new(&engine, bytes) {
		Ok(m) => m,
		Err(e) => {
			return RunResult {
				status: format!("module error: {e}"),
				stdout: String::new(),
			};
		}
	};
	run_entry(&engine, &module, stdin)
}

/// Instantiate a pre-compiled module and run `_entry` once with stdin from a slice,
/// capturing stdout. Split out of `run_wasm` so a benchmark can re-instantiate a
/// module that was cranelift-compiled once, keeping JIT compilation out of the
/// timed loop.
pub fn run_entry(engine: &Engine, module: &Module, stdin: &[u8]) -> RunResult {
	run_with(engine, module, Box::new(BufferedIo::new(stdin)))
}

/// Compile + instantiate `bytes` and run `_entry` once, streaming stdout/stderr to
/// the process and reading stdin from it (the `cli`'s `pluma run app.wasm` path).
/// Returns the process exit code; on failure the program's abort message is already
/// on stderr.
pub fn run_streaming(bytes: &[u8]) -> i32 {
	let engine = engine();
	let module = match Module::new(&engine, bytes) {
		Ok(m) => m,
		Err(e) => {
			eprintln!("Could not load wasm module: {e}");
			return 1;
		}
	};
	let result = run_with(&engine, &module, Box::new(StdioIo::new()));
	match result.status.as_str() {
		"ok" => 0,
		other => {
			// `other` is "runtime error: <msg>" — print the program's own message
			// bare to stderr (mirrors the VM surfacing an abort), then exit nonzero.
			let msg = other.strip_prefix("runtime error: ").unwrap_or(other);
			eprintln!("{msg}");
			1
		}
	}
}

/// Drive `_entry` once through the given sink and report the status. `_entry`'s
/// returned `err e` doubles as the exit status (mirrors `vm::VM::run`); an
/// `io.fail` trap surfaces its stashed message.
fn run_with(engine: &Engine, module: &Module, io: Box<dyn HostIo>) -> RunResult {
	let (mut store, instance) = match instantiate_module(engine, module, io) {
		Ok(x) => x,
		Err(status) => {
			return RunResult {
				status,
				stdout: String::new(),
			};
		}
	};
	let entry = instance
		.get_func(&mut store, "_entry")
		.expect("_entry export");
	// Every Pluma function takes an implicit closure-env param first; the entry
	// ignores it, so pass null.
	let mut results = vec![Val::AnyRef(None)];
	let status = match entry.call(&mut store, &[Val::AnyRef(None)], &mut results) {
		// `main`'s return value doubles as the exit status: a returned `err e`
		// aborts with `e` (mirrors `vm::VM::run`).
		Ok(_) => match err_message(&mut store, &results[0]) {
			Some(msg) => format!("runtime error: {msg}"),
			None => "ok".to_string(),
		},
		// A trap with a stashed `io.fail` message is a program-controlled abort;
		// surface its message (matching the VM) rather than the wasm backtrace.
		Err(e) => match store.data().fail.clone() {
			Some(msg) => format!("runtime error: {msg}"),
			None => format!("runtime error: {e}"),
		},
	};
	let stdout = store.data().io.captured_stdout();
	RunResult { status, stdout }
}

/// A collecting (deferred-reference-counting) engine for the bench: the timed loop
/// allocates a record per iteration, which the default null collector would never
/// free (OOM). The short-lived records are reclaimed within each `_entry` call.
pub fn bench_engine() -> Engine {
	let mut config = Config::new();
	config.wasm_reference_types(true);
	config.wasm_function_references(true);
	config.wasm_gc(true);
	config.wasm_tail_call(true);
	config.collector(wasmtime::Collector::DeferredReferenceCounting);
	Engine::new(&config).expect("bench engine")
}

/// If `val` is an `err e` result variant, return `e` formatted (the program's
/// abort message); otherwise `None`.
fn err_message(store: &mut impl AsContextMut, val: &Val) -> Option<String> {
	let Val::AnyRef(Some(r)) = val else {
		return None;
	};
	let s = r.as_struct(&mut *store).ok()??;
	if !matches!(s.field(&mut *store, 0).ok()?, Val::I32(TAG_VARIANT)) {
		return None;
	}
	// field 2 is the display name "enum.variant"; field 3 the payload.
	let name_val = s.field(&mut *store, 2).ok()?;
	let name = format_value(store, &name_val);
	if name.rsplit('.').next() != Some("err") {
		return None;
	}
	let payload_val = s.field(&mut *store, 3).ok()?;
	let payload = format_elems(store, &payload_val);
	(payload.len() == 1).then(|| payload[0].clone())
}
