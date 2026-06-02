// The WasmGC runtime host. Instantiates an emitted module in wasmtime, supplies the
// `pluma.*` host imports (print/io/net/float_to_str/math), and runs `_entry`.
//
// Per ABI.md Phase 1, the host imports traffic only **bytes + scalars + handles**
// across the boundary — byte payloads cross through the module's exported scratch
// `"memory"` (read/written via `read_scratch`/`write_scratch`); the host never reads
// or builds a GC `$value` field. That keeps the boundary engine-neutral (the same
// ABI a V8-on-server or browser JS shim needs — neither can reflect WasmGC structs).
// The one remaining GC read is `err_message`/`format_value`, which inspects `_entry`'s
// *return* value (a `runtime error: <msg>` surface, not a host import) — a Phase-2
// item to marshal once the engine swaps off wasmtime.
//
// Two front doors share one engine + one set of host imports:
//   - `run_wasm`/`run_entry` — **buffered** (stdout captured, stderr dropped,
//     stdin fed from a byte slice). The `conformance` crate's differential path.
//   - `run_streaming` — **process stdio** (stdout/stderr streamed live, stdin read
//     from the process). The `cli`'s `pluma run app.wasm` path.
// The only thing that differs is the `HostIo` sink behind `HostState`; every host
// import is identical, so the conformance gate tests exactly the runtime the CLI ships.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::time::Duration;

use polling::{Event, Events, Poller};
use wasmtime::{
	AnyRef, AsContextMut, Caller, Config, Engine, Extern, FuncType, Instance, Linker, Memory, Module,
	Rooted, Store, Val, ValType,
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
const TAG_EXTERN: i32 = 19;

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
	/// Bytes a read op produced that didn't fit the caller's first `dst` buffer; the
	/// wasm side then reserves the true size and drains this via `__io_copyout`. Empty
	/// on the common (fits-first-try) path. (ABI.md Phase 1, the read overflow path.)
	read_stash: Vec<u8>,
	/// `core.net` runtime state: the socket table + the I/O reactor (the host-side
	/// analogue of `vm::net::NetState`).
	net: HostNet,
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
	// Collector defaults to null (allocate, never collect): conformance fixtures and
	// `pluma run app.wasm` are tiny, short-lived programs, so never collecting is the
	// fastest option. (Null also used to dodge a wasmtime 30 deferred-reference-
	// counting panic ("invalid VMGcKind"); that bug is fixed as of wasmtime 45.)
	//
	// `PLUMA_WASM_GC=drc` opts into the real deferred-reference-counting collector —
	// the configuration a long-lived deploy actually requires (null OOMs once the
	// heap fills). The competition harness sets it so its numbers reflect a real GC,
	// not a best-case bump allocator. Default stays null, so nothing else changes.
	let collector = match std::env::var("PLUMA_WASM_GC").as_deref() {
		Ok("drc") => wasmtime::Collector::DeferredReferenceCounting,
		_ => wasmtime::Collector::Null,
	};
	config.collector(collector);
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
	// A small int rides as an `i31ref` immediate (no `$value` struct); see
	// `notes/I31.md`. Format it as the decimal int it represents.
	if let Some(i31) = any.as_i31(&mut *store).expect("as_i31") {
		return i31.get_i32().to_string();
	}
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
		// A host handle: opaque, never structurally printed (mirrors `__tostring`).
		// No Phase-1 value reaches this — it's here so the entry-return format path
		// stays total once DOM/fetch (Phase 3) start producing externs.
		TAG_EXTERN => "<extern>".to_string(),
		TAG_DICT => {
			// `{k: v, ...}` in insertion order. The `$dict` is a persistent hash-trie
			// (`{ tag, root, next_seq }`); walk it to collect `(seq, key, value)`, then
			// order by `seq`. (Mostly a debug path: well-typed `print`/`to-string`
			// stringify dicts wasm-side; this is the host-side `format` fallback.)
			let root = s.field(&mut *store, 1).expect("dict root");
			let mut entries: Vec<(i64, String, String)> = Vec::new();
			collect_dict(store, &root, &mut entries);
			entries.sort_by_key(|(seq, _, _)| *seq);
			let pairs: Vec<String> = entries
				.iter()
				.map(|(_, k, v)| format!("{k}: {v}"))
				.collect();
			format!("{{{}}}", pairs.join(", "))
		}
		other => format!("<tag {other}>"),
	}
}

/// Walk a `$dict`'s persistent hash-trie, collecting `(seq, key, value)` for each
/// entry (key/value formatted recursively). `node` is a `$dnode` ref or null. A
/// branch (`kids` non-null, field 1) recurses into its child slots; a leaf
/// (`ents` non-null, field 2) reads each `$tuple(key, value, seq)`.
fn collect_dict(store: &mut impl AsContextMut, node: &Val, out: &mut Vec<(i64, String, String)>) {
	let Val::AnyRef(Some(r)) = node else {
		return; // empty subtree
	};
	let s = r
		.as_struct(&mut *store)
		.expect("as_struct")
		.expect("a $dnode");
	match s.field(&mut *store, 1).expect("dnode kids") {
		// branch: recurse into each of the (up to 16) child slots.
		Val::AnyRef(Some(kids)) => {
			let a = kids
				.as_array(&mut *store)
				.expect("as_array")
				.expect("kids array");
			let n = a.len(&mut *store).expect("array len");
			for i in 0..n {
				let child = a.get(&mut *store, i).expect("array get");
				collect_dict(store, &child, out);
			}
		}
		// leaf: each `ents` element is a `$tuple(key, value, seq)`.
		_ => {
			let ents = match s.field(&mut *store, 2).expect("dnode ents") {
				Val::AnyRef(Some(arr)) => arr
					.as_array(&mut *store)
					.expect("as_array")
					.expect("ents array"),
				_ => return,
			};
			let n = ents.len(&mut *store).expect("array len");
			for i in 0..n {
				let Val::AnyRef(Some(tr)) = ents.get(&mut *store, i).expect("array get") else {
					continue;
				};
				let ts = tr
					.as_struct(&mut *store)
					.expect("as_struct")
					.expect("a $tuple");
				let elems = match ts.field(&mut *store, 1).expect("tuple elems") {
					Val::AnyRef(Some(arr)) => arr
						.as_array(&mut *store)
						.expect("as_array")
						.expect("elems array"),
					_ => continue,
				};
				let key = elems.get(&mut *store, 0).expect("entry key");
				let val = elems.get(&mut *store, 1).expect("entry value");
				// seq is a small int: an `i31ref` immediate when small (the common case),
				// else a heap `$int` (field 1 = i64).
				let seq = match elems.get(&mut *store, 2).expect("entry seq") {
					Val::AnyRef(Some(sr)) => {
						if let Some(i31) = sr.as_i31(&mut *store).expect("as_i31") {
							i31.get_i32() as i64
						} else {
							let ss = sr.as_struct(&mut *store).expect("as_struct").expect("$int");
							match ss.field(&mut *store, 1).expect("seq i64") {
								Val::I64(n) => n,
								_ => 0,
							}
						}
					}
					_ => 0,
				};
				let ks = format_value(store, &key);
				let vs = format_value(store, &val);
				out.push((seq, ks, vs));
			}
		}
	}
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

// --------------------------------------------------------------------------
// The marshalling-ABI scratch helpers (ABI.md Phase 1). Host imports traffic byte
// payloads through the module's exported `"memory"` linear memory instead of
// reflecting GC `$value` fields — the engine-neutral boundary every real deploy
// host (V8, a browser JS shim) needs.
// --------------------------------------------------------------------------

/// The module's exported scratch `"memory"`. Every emitted module exports it.
fn scratch_mem(caller: &mut Caller<HostState>) -> Memory {
	match caller.get_export("memory") {
		Some(Extern::Memory(m)) => m,
		_ => panic!("module is missing its exported `memory`"),
	}
}

/// Read `len` bytes of scratch memory at `ptr` into an owned `Vec` (releasing the
/// memory borrow before the caller mutates `HostState`).
fn read_scratch(caller: &mut Caller<HostState>, ptr: i32, len: i32) -> Vec<u8> {
	let mem = scratch_mem(caller);
	let mut buf = vec![0u8; len.max(0) as usize];
	mem
		.read(&*caller, ptr as usize, &mut buf)
		.expect("scratch read in bounds");
	buf
}

/// Write `data` into scratch memory at `ptr` (the caller has reserved the room).
fn write_scratch(caller: &mut Caller<HostState>, ptr: i32, data: &[u8]) {
	let mem = scratch_mem(caller);
	mem
		.write(&mut *caller, ptr as usize, data)
		.expect("scratch write in bounds");
}

/// Extract an `i32` host-call argument.
fn arg_i32(v: &Val) -> i32 {
	match v {
		Val::I32(n) => *n,
		o => panic!("expected i32 arg: {o:?}"),
	}
}

/// Deliver a read's `bytes` to the caller's `(dst, cap)` buffer: write them into
/// scratch and return the length if they fit; otherwise stash them (the wasm side
/// reserves the true size and drains via `__io_copyout`) and return the over-`cap`
/// length. Returns the true byte count either way — the wasm side compares it to
/// `cap` to take the overflow branch.
fn deliver_read(caller: &mut Caller<HostState>, dst: i32, cap: i32, bytes: Vec<u8>) -> i32 {
	let len = bytes.len();
	if len <= cap.max(0) as usize {
		write_scratch(caller, dst, &bytes);
	} else {
		caller.data_mut().read_stash = bytes;
	}
	len as i32
}

/// Shape a scalar `NetRet` (a socket id / write count / nothing) into the `(status,
/// n)` pair a net import returns: 0 ok, 1 would-block, 2 err (message → `last_error`,
/// read back via `io-last-error`, exactly like `core.io`).
fn net_scalar(caller: &mut Caller<HostState>, ret: NetRet) -> (i32, i32) {
	match ret {
		NetRet::OkInt(v) => (0, v),
		NetRet::OkNothing => (0, 0),
		NetRet::WouldBlock => (1, 0),
		NetRet::Err(e) => {
			set_io_err(caller, e);
			(2, 0)
		}
		NetRet::OkBytes(_) | NetRet::OkStr(_) => unreachable!("net_scalar on a byte-returning op"),
	}
}

/// Shape a byte-returning `NetRet` (`net-read` bytes / `net-local-addr` string) into
/// `(status, len)`, writing the payload into scratch at `dst` (truncated to `cap` —
/// `net-read` already bounds by `max == cap`, addresses are short).
fn net_bytes(caller: &mut Caller<HostState>, dst: i32, cap: i32, ret: NetRet) -> (i32, i32) {
	let bytes = match ret {
		NetRet::OkBytes(b) => b,
		NetRet::OkStr(s) => s.into_bytes(),
		NetRet::WouldBlock => return (1, 0),
		NetRet::Err(e) => {
			set_io_err(caller, e);
			return (2, 0);
		}
		NetRet::OkInt(_) | NetRet::OkNothing => unreachable!("net_bytes on a scalar op"),
	};
	let len = bytes.len().min(cap.max(0) as usize);
	write_scratch(caller, dst, &bytes[..len]);
	(0, len as i32)
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
			read_stash: Vec::new(),
			net: HostNet::default(),
		},
	);
	let mut linker: Linker<HostState> = Linker::new(engine);
	// The byte-payload writers (ABI.md Phase 1). Each takes `(i32 ptr, i32 len)` into
	// the scratch memory — wasm has already rendered the value's bytes there (via
	// `__tostring` for the formatted writers, or the raw `$bytes` backing for the
	// `*-bytes` pair), so the host just reads the slice. `print`/`print-err` append a
	// newline; the `*-err` variants target stderr. The host no longer reflects any GC
	// field, so this boundary runs unchanged under V8 / a browser shim.
	let write_ty = FuncType::new(engine, [ValType::I32, ValType::I32], []);
	for (name, to_err, newline) in [
		("print", false, true),
		("io-print", false, true),
		("io-write", false, false),
		("io-print-err", true, true),
		("io-write-err", true, false),
		("io-write-bytes", false, false),
		("io-write-err-bytes", true, false),
	] {
		linker
			.func_new(
				"pluma",
				name,
				write_ty.clone(),
				move |mut caller, args, _results| {
					let (ptr, len) = (arg_i32(&args[0]), arg_i32(&args[1]));
					let mut bytes = read_scratch(&mut caller, ptr, len);
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
	// io.fail msg : read the pre-rendered message out of scratch, stash it, then trap.
	// The runner reads the message back to form the `runtime error: <msg>` status
	// (mirrors the VM's abort).
	linker
		.func_new(
			"pluma",
			"io-fail",
			write_ty.clone(),
			|mut caller, args, _results| {
				let (ptr, len) = (arg_i32(&args[0]), arg_i32(&args[1]));
				let bytes = read_scratch(&mut caller, ptr, len);
				caller.data_mut().fail = Some(String::from_utf8_lossy(&bytes).into_owned());
				Err(wasmtime::Error::msg("io.fail"))
			},
		)
		.expect("define io-fail");
	// float_to_str : (f64, i32 ptr, i32 cap) -> i32 len. Format the float as
	// `vm::Value`'s Display does, write its UTF-8 bytes into scratch at `ptr` (≤ cap),
	// return the length. A float renders to ≤ 24 bytes, so the wasm side's 32-byte cap
	// never overflows. (A browser target would delegate to JS `String(x)` similarly.)
	let f2s_ty = FuncType::new(
		engine,
		[ValType::F64, ValType::I32, ValType::I32],
		[ValType::I32],
	);
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
				let (ptr, cap) = (arg_i32(&args[1]), arg_i32(&args[2]));
				let bytes = s.as_bytes();
				if bytes.len() <= cap as usize {
					write_scratch(&mut caller, ptr, bytes);
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

	// The marshalled `core.io` host imports (ABI.md Phase 1). Path/data args arrive as
	// `(ptr, len)` byte slices in scratch; reads write their result back into a caller
	// `(dst, cap)` buffer (a `len > cap` overflow stashes for `__io_copyout`). The host
	// no longer reflects or builds any GC `$value` — wasm shapes the `i32` result.
	let io2_ty = FuncType::new(engine, [ValType::I32, ValType::I32], [ValType::I32]);
	let io4_ty = FuncType::new(
		engine,
		[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
		[ValType::I32],
	);
	let copyout_ty = FuncType::new(engine, [ValType::I32], []);

	// read-file / read-file-bytes : (path, plen, dst, cap) -> len (neg ⇒ err). Text
	// reads validate UTF-8 (matching the VM); both deliver the bytes via the read
	// buffer + overflow stash.
	for (name, as_bytes) in [("io-read-file", false), ("io-read-file-bytes", true)] {
		linker
			.func_new(
				"pluma",
				name,
				io4_ty.clone(),
				move |mut caller, args, results| {
					let (pp, pl) = (arg_i32(&args[0]), arg_i32(&args[1]));
					let (dst, cap) = (arg_i32(&args[2]), arg_i32(&args[3]));
					let path = String::from_utf8_lossy(&read_scratch(&mut caller, pp, pl)).into_owned();
					let res = if as_bytes {
						std::fs::read(&path)
					} else {
						std::fs::read_to_string(&path).map(String::into_bytes)
					};
					results[0] = match res {
						Ok(bytes) => Val::I32(deliver_read(&mut caller, dst, cap, bytes)),
						Err(e) => {
							set_io_err(&mut caller, e.to_string());
							Val::I32(-1)
						}
					};
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// write-file / append-file (+ bytes variants) : (path, plen, data, dlen) -> status
	// (0 ok / non-0 err). wasm already encoded `data`'s bytes, so the text/bytes
	// variants share a closure — only append-vs-truncate differs.
	for (name, append) in [
		("io-write-file", false),
		("io-append-file", true),
		("io-write-file-bytes", false),
		("io-append-file-bytes", true),
	] {
		linker
			.func_new(
				"pluma",
				name,
				io4_ty.clone(),
				move |mut caller, args, results| {
					let (pp, pl) = (arg_i32(&args[0]), arg_i32(&args[1]));
					let (dp, dl) = (arg_i32(&args[2]), arg_i32(&args[3]));
					let path = String::from_utf8_lossy(&read_scratch(&mut caller, pp, pl)).into_owned();
					let data = read_scratch(&mut caller, dp, dl);
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
						Ok(()) => Val::I32(0),
						Err(e) => {
							set_io_err(&mut caller, e.to_string());
							Val::I32(2)
						}
					};
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// delete-file / make-dir (mkdir -p) : (path, plen) -> status.
	for (name, is_mkdir) in [("io-delete-file", false), ("io-make-dir", true)] {
		linker
			.func_new(
				"pluma",
				name,
				io2_ty.clone(),
				move |mut caller, args, results| {
					let (pp, pl) = (arg_i32(&args[0]), arg_i32(&args[1]));
					let path = String::from_utf8_lossy(&read_scratch(&mut caller, pp, pl)).into_owned();
					let res = if is_mkdir {
						std::fs::create_dir_all(&path)
					} else {
						std::fs::remove_file(&path)
					};
					results[0] = match res {
						Ok(()) => Val::I32(0),
						Err(e) => {
							set_io_err(&mut caller, e.to_string());
							Val::I32(2)
						}
					};
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// file-exists / is-dir : (path, plen) -> bool (0/1).
	for (name, is_dir) in [("io-file-exists", false), ("io-is-dir", true)] {
		linker
			.func_new(
				"pluma",
				name,
				io2_ty.clone(),
				move |mut caller, args, results| {
					let (pp, pl) = (arg_i32(&args[0]), arg_i32(&args[1]));
					let path = String::from_utf8_lossy(&read_scratch(&mut caller, pp, pl)).into_owned();
					let p = std::path::Path::new(&path);
					let b = if is_dir { p.is_dir() } else { p.exists() };
					results[0] = Val::I32(b as i32);
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}

	// read-dir : (path, plen, dst, cap) -> len (neg ⇒ err). Entry names only, sorted
	// (VM parity), NUL-terminated so the wasm side can split them into a `$list`.
	linker
		.func_new(
			"pluma",
			"io-read-dir",
			io4_ty.clone(),
			|mut caller, args, results| {
				let (pp, pl) = (arg_i32(&args[0]), arg_i32(&args[1]));
				let (dst, cap) = (arg_i32(&args[2]), arg_i32(&args[3]));
				let path = String::from_utf8_lossy(&read_scratch(&mut caller, pp, pl)).into_owned();
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
								Val::I32(-1)
							}
							None => {
								names.sort();
								let mut blob = Vec::new();
								for n in &names {
									blob.extend_from_slice(n.as_bytes());
									blob.push(0); // NUL terminator
								}
								Val::I32(deliver_read(&mut caller, dst, cap, blob))
							}
						}
					}
					Err(e) => {
						set_io_err(&mut caller, e.to_string());
						Val::I32(-1)
					}
				};
				Ok(())
			},
		)
		.expect("define io-read-dir");

	// read / read-all / read-all-bytes : (dst, cap) -> len (neg ⇒ err).
	linker
		.func_new(
			"pluma",
			"io-read",
			io2_ty.clone(),
			|mut caller, args, results| {
				let (dst, cap) = (arg_i32(&args[0]), arg_i32(&args[1]));
				let line = caller.data_mut().io.read_line();
				results[0] = match line {
					Some(line) => Val::I32(deliver_read(&mut caller, dst, cap, line.into_bytes())),
					None => {
						set_io_err(&mut caller, "EOF".to_string());
						Val::I32(-1)
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
			io2_ty.clone(),
			|mut caller, args, results| {
				let (dst, cap) = (arg_i32(&args[0]), arg_i32(&args[1]));
				let bytes = caller.data_mut().io.read_rest();
				let s = String::from_utf8_lossy(&bytes).into_owned();
				results[0] = Val::I32(deliver_read(&mut caller, dst, cap, s.into_bytes()));
				Ok(())
			},
		)
		.expect("define io-read-all");
	linker
		.func_new(
			"pluma",
			"io-read-all-bytes",
			io2_ty.clone(),
			|mut caller, args, results| {
				let (dst, cap) = (arg_i32(&args[0]), arg_i32(&args[1]));
				let bytes = caller.data_mut().io.read_rest();
				results[0] = Val::I32(deliver_read(&mut caller, dst, cap, bytes));
				Ok(())
			},
		)
		.expect("define io-read-all-bytes");

	// io-last-error : (dst, cap) -> len. The message the last failed io call stashed,
	// written into scratch (truncated to `cap` — errno strings are short, so no stash).
	linker
		.func_new(
			"pluma",
			"io-last-error",
			io2_ty.clone(),
			|mut caller, args, results| {
				let (dst, cap) = (arg_i32(&args[0]), arg_i32(&args[1]));
				let msg = caller.data().last_error.clone();
				let bytes = msg.as_bytes();
				let len = bytes.len().min(cap.max(0) as usize);
				write_scratch(&mut caller, dst, &bytes[..len]);
				results[0] = Val::I32(len as i32);
				Ok(())
			},
		)
		.expect("define io-last-error");

	// __io_copyout : (dst) -> () — drain the read stash into scratch at `dst` (the
	// overflow path, after the wasm side reserved the true size).
	linker
		.func_new(
			"pluma",
			"io-copyout",
			copyout_ty,
			|mut caller, args, _results| {
				let dst = arg_i32(&args[0]);
				let stash = std::mem::take(&mut caller.data_mut().read_stash);
				write_scratch(&mut caller, dst, &stash);
				Ok(())
			},
		)
		.expect("define io-copyout");

	// The marshalled `core.net` host imports (ABI.md Phase 1). Addresses + data cross
	// as `(ptr, len)` scratch slices; socket ids are unboxed `i32`s; each op returns a
	// `(status, n)` pair (`net-close` just `status`). The host no longer reflects or
	// builds GC `$value`s — wasm shapes the result via `__io_result` (reusing the
	// `core.io` `ok`/`err` + `io-last-error` channel; net errors set `last_error`).
	let net_listen_ty = FuncType::new(
		engine,
		[ValType::I32, ValType::I32],
		[ValType::I32, ValType::I32],
	);
	let net_close_ty = FuncType::new(engine, [ValType::I32], [ValType::I32]);
	let net_local_ty = FuncType::new(
		engine,
		[ValType::I32, ValType::I32, ValType::I32],
		[ValType::I32, ValType::I32],
	);
	let net_rw_ty = FuncType::new(
		engine,
		[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
		[ValType::I32, ValType::I32],
	);
	// net-listen / net-connect : (addr, alen) -> (status, socket-id).
	for (name, connect) in [("net-listen", false), ("net-connect", true)] {
		linker
			.func_new(
				"pluma",
				name,
				net_listen_ty.clone(),
				move |mut caller, args, results| {
					let (ap, al) = (arg_i32(&args[0]), arg_i32(&args[1]));
					let addr = String::from_utf8_lossy(&read_scratch(&mut caller, ap, al)).into_owned();
					let ret = if connect {
						caller.data_mut().net.connect(&addr)
					} else {
						caller.data_mut().net.listen(&addr)
					};
					let (status, n) = net_scalar(&mut caller, ret);
					results[0] = Val::I32(status);
					results[1] = Val::I32(n);
					Ok(())
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}
	// net-close : (id) -> status.
	linker
		.func_new(
			"pluma",
			"net-close",
			net_close_ty,
			|mut caller, args, results| {
				let id = arg_i32(&args[0]) as u32;
				let ret = caller.data_mut().net.close(id);
				let (status, _) = net_scalar(&mut caller, ret);
				results[0] = Val::I32(status);
				Ok(())
			},
		)
		.expect("define net-close");
	// net-local-addr : (id, dst, cap) -> (status, len). Address string into scratch.
	linker
		.func_new(
			"pluma",
			"net-local-addr",
			net_local_ty,
			|mut caller, args, results| {
				let id = arg_i32(&args[0]) as u32;
				let (dst, cap) = (arg_i32(&args[1]), arg_i32(&args[2]));
				let ret = caller.data_mut().net.local_addr(id);
				let (status, len) = net_bytes(&mut caller, dst, cap, ret);
				results[0] = Val::I32(status);
				results[1] = Val::I32(len);
				Ok(())
			},
		)
		.expect("define net-local-addr");
	// net-accept : (fid, listener-id) -> (status, conn-id) | would-block.
	linker
		.func_new(
			"pluma",
			"net-accept",
			net_listen_ty.clone(),
			|mut caller, args, results| {
				let fid = arg_i32(&args[0]);
				let lid = arg_i32(&args[1]) as u32;
				let ret = caller.data_mut().net.try_accept(fid, lid);
				let (status, n) = net_scalar(&mut caller, ret);
				results[0] = Val::I32(status);
				results[1] = Val::I32(n);
				Ok(())
			},
		)
		.expect("define net-accept");
	// net-read : (fid, conn, dst, cap) -> (status, len) | would-block. cap == the
	// requested max, so the read never exceeds it (no stash/overflow).
	linker
		.func_new(
			"pluma",
			"net-read",
			net_rw_ty.clone(),
			|mut caller, args, results| {
				let fid = arg_i32(&args[0]);
				let cid = arg_i32(&args[1]) as u32;
				let (dst, cap) = (arg_i32(&args[2]), arg_i32(&args[3]));
				let ret = caller
					.data_mut()
					.net
					.try_read(fid, cid, cap.max(0) as usize);
				let (status, len) = net_bytes(&mut caller, dst, cap, ret);
				results[0] = Val::I32(status);
				results[1] = Val::I32(len);
				Ok(())
			},
		)
		.expect("define net-read");
	// net-write : (fid, conn, src, len) -> (status, n) | would-block.
	linker
		.func_new(
			"pluma",
			"net-write",
			net_rw_ty,
			|mut caller, args, results| {
				let fid = arg_i32(&args[0]);
				let cid = arg_i32(&args[1]) as u32;
				let (src, len) = (arg_i32(&args[2]), arg_i32(&args[3]));
				let data = read_scratch(&mut caller, src, len);
				let ret = caller.data_mut().net.try_write(fid, cid, &data);
				let (status, n) = net_scalar(&mut caller, ret);
				results[0] = Val::I32(status);
				results[1] = Val::I32(n);
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
