// The WasmGC runtime host. Instantiates an emitted module under V8, supplies the
// `pluma.*` host imports (print/io/net/float_to_str/math), and runs `_entry`.
//
// Per ABI.md, the host imports traffic only **bytes + scalars + handles** across the
// boundary — byte payloads cross through the module's exported scratch `"memory"`; the
// host never reads or builds a GC `$value` field. That engine-neutral marshalling ABI
// is what lets a stock JS engine like V8 — which cannot reflect WasmGC structs — run
// the artifact at all, and is the same boundary a browser JS shim would mirror. Even
// the program-failure surface is reflection-free: the host shuttles `_entry`'s opaque
// return ref into the module's `__entry_error` export, which renders any `result.err`
// message into scratch.
//
// This file holds the engine-independent core — the `HostIo` sinks, `HostState`, and
// the `core.net` reactor (`HostNet`) — that the V8 runner in `v8host.rs` drives. That
// runner has two front doors over one set of host imports:
//   - `run_wasm_v8` — **buffered** (stdout captured, stderr dropped, stdin fed from a
//     byte slice). The `conformance` crate's differential path.
//   - `run_streaming_v8` — **process stdio** (stdout/stderr streamed live, stdin read
//     from the process). The `cli`'s `pluma run` path.
// The only thing that differs is the `HostIo` sink behind `HostState`, so the
// conformance gate tests exactly the runtime the CLI ships.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::time::Duration;

use polling::{Event, Events, Poller};

// The V8 backend (ABI.md Phase 2): instantiates the WasmGC artifact under V8 over the
// marshalling ABI. Reuses this module's engine-independent core (`HostState`/`HostNet`/
// `NetRet`/`BufferedIo`/`read_line_from`) — a child module sees its ancestors' private
// items, so nothing here needs `pub`.
mod v8host;
pub use v8host::{bench_exec_v8, run_streaming_v8, run_wasm_v8};

/// A program's observable result: exit status + captured stdout. (The streaming runner
/// returns an empty `stdout` — it streamed live to the process — and the caller uses
/// `status` for the exit code.)
#[derive(Clone, PartialEq, Eq)]
pub struct RunResult {
	pub status: String,
	pub stdout: String,
}

// --------------------------------------------------------------------------
// The stdio sink. `HostState` is non-generic (the V8 import callbacks reach it through
// a raw `Ctx` pointer); the buffered-vs-streaming choice is a trait object.
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
