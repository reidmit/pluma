// The WasmGC runtime host. Instantiates an emitted module under V8, supplies the
// `pluma.*` host imports (print/io/net/float_to_str/math), and runs `_entry`.
//
// The host imports traffic only **bytes + scalars + handles** across the
// boundary — byte payloads cross through the module's exported scratch `"memory"`; the
// host never reads or builds a GC `$value` field. That engine-neutral marshalling ABI
// is what lets a stock JS engine like V8 — which cannot reflect WasmGC structs — run
// the artifact at all, and is the same boundary a browser JS shim would mirror. Even
// the program-failure surface is reflection-free: the host shuttles `_entry`'s opaque
// return ref into the module's `__entry_error` export, which renders any `result.err`
// message into scratch.
//
// This file holds the engine-independent core — the `HostIo` sinks, `HostState`, and
// the `std/sys/net` reactor (`HostNet`) — that the V8 runner in `v8host.rs` drives. That
// runner has three front doors over one set of host imports, differing only in the
// `HostIo` sink behind `HostState` — so every door tests the exact runtime the CLI ships:
//   - `run_streaming_v8` — **process stdio** (stdout/stderr streamed live, stdin read
//     from the process). The `cli`'s `pluma run` / `pluma test` path.
//   - `run_wasm_v8_captured` — **buffered, both streams** (status + stdout + stderr
//     captured, stdin fed from a byte slice). The `tests/run` snapshot suite's path.
//   - `run_wasm_v8` — **buffered, stdout only** (stderr dropped). A minimal entry point
//     (the `v8smoke` example).

use std::io::{Read, Write};

use db::HostDb;
use net::HostNet;
use offload::Reactor;

// The `std/sys/net` host-side socket table (`HostNet`/`NetRet`), kept in its own
// engine-independent module; `HostState` holds one for the run.
mod net;

// The shared blocking-I/O offload subsystem (host/src/offload.rs): the `Reactor` (one poller for
// both socket readiness and worker completion) + the `BlockingPool` of worker threads.
// `std/sys/net` parks socket reads on it; offload clients (fs, db, …) submit blocking jobs.
mod offload;

// Engine-independent `std/sys/fs` ops (one op-code dispatch), shared by the async pool
// path and the synchronous `-sync` path so the two can't drift.
mod fsop;

// Engine-independent `std/sys/process` subprocess ops. Rides the same op-code dispatch as
// `fsop` (the offload callbacks route op-codes >= `procop::op::RUN` here).
mod procop;

// Engine-independent `std/sys/db` (embedded SQLite): the pinned worker owning the
// `rusqlite::Connection`s + the value/row wire codec, an offload client of `offload.rs`.
mod db;

// The V8 backend: instantiates the WasmGC artifact under V8 over the
// marshalling ABI. Reuses this crate's engine-independent core (`HostState`/`HostNet`/
// `NetRet`/`BufferedIo`/`read_line_from`) — a descendant module sees its ancestors'
// private items, so nothing here needs `pub`.
mod v8host;
pub use v8host::{run_streaming_v8, run_test_v8, run_wasm_v8, run_wasm_v8_captured};

/// A program's observable result: exit status + captured stdout. (The streaming runner
/// returns an empty `stdout` — it streamed live to the process — and the caller uses
/// `status` for the exit code.)
#[derive(Clone, PartialEq, Eq)]
pub struct RunResult {
	pub status: String,
	pub stdout: String,
}

/// A program's observable result with stderr kept separate — the snapshot-suite
/// shape (`tests/run`). Unlike `RunResult` (status + stdout only), the snapshot
/// harness pins stderr too, so it needs a runner that captures both streams.
#[derive(Clone, PartialEq, Eq)]
pub struct RunCapture {
	pub status: String,
	pub stdout: String,
	pub stderr: String,
}

// --------------------------------------------------------------------------
// The stdio sink. `HostState` is non-generic (the V8 import callbacks reach it through
// a raw `Ctx` pointer); the buffered-vs-streaming choice is a trait object.
// --------------------------------------------------------------------------

/// Where the host's stdout/stderr go and where stdin comes from. The reads use
/// `read_line` semantics (line up to `\n`, trailing `\r` stripped, `None` at
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
	/// The stderr collected so far, for the snapshot path (`run_wasm_v8_captured`).
	/// Sinks that drop or stream stderr return `""`.
	fn captured_stderr(&self) -> String {
		String::new()
	}
}

/// Captures stdout, drops stderr, and reads stdin from a fixed byte buffer. The
/// stdout-only buffered sink (`run_wasm_v8`).
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

/// Like `BufferedIo`, but also captures stderr — the snapshot suite (`tests/run`)
/// pins both streams.
struct CapturingIo {
	out: Vec<u8>,
	err: Vec<u8>,
	stdin: Vec<u8>,
	stdin_pos: usize,
}

impl CapturingIo {
	fn new(stdin: &[u8]) -> Self {
		CapturingIo {
			out: Vec::new(),
			err: Vec::new(),
			stdin: stdin.to_vec(),
			stdin_pos: 0,
		}
	}
}

impl HostIo for CapturingIo {
	fn write_out(&mut self, bytes: &[u8]) {
		self.out.extend_from_slice(bytes);
	}
	fn write_err(&mut self, bytes: &[u8]) {
		self.err.extend_from_slice(bytes);
	}
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
	fn captured_stderr(&self) -> String {
		String::from_utf8_lossy(&self.err).into_owned()
	}
}

/// Streams stdout/stderr live to the process and reads stdin from it. Each write
/// flushes so output appears promptly (current artifacts are short-lived; a
/// long-running net server may revisit buffering). Reads pull one line at a time
/// off a buffered stdin so an interactive REPL sees each line as it's entered,
/// rather than blocking until the whole stream reaches EOF.
struct StdioIo {
	stdin: Option<std::io::BufReader<std::io::Stdin>>,
	/// When true, stdin reads return EOF without ever touching the process stdin.
	/// `pluma test` uses this so a case that calls `io.read` gets EOF immediately
	/// rather than blocking on the terminal — a test runner has no one interactive
	/// stdin to hand a suite of cases, matching the empty-stdin the snapshot harness
	/// fed these same programs.
	stdin_closed: bool,
}

impl StdioIo {
	fn new() -> Self {
		StdioIo {
			stdin: None,
			stdin_closed: false,
		}
	}
	fn reader(&mut self) -> &mut std::io::BufReader<std::io::Stdin> {
		self
			.stdin
			.get_or_insert_with(|| std::io::BufReader::new(std::io::stdin()))
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
		use std::io::BufRead;
		if self.stdin_closed {
			return None;
		}
		let mut buf = Vec::new();
		match self.reader().read_until(b'\n', &mut buf) {
			Ok(0) => None,
			Ok(_) => {
				if buf.last() == Some(&b'\n') {
					buf.pop();
				}
				if buf.last() == Some(&b'\r') {
					buf.pop();
				}
				Some(String::from_utf8_lossy(&buf).into_owned())
			}
			Err(_) => None,
		}
	}
	fn read_rest(&mut self) -> Vec<u8> {
		if self.stdin_closed {
			return Vec::new();
		}
		let mut buf = Vec::new();
		let _ = self.reader().read_to_end(&mut buf);
		buf
	}
}

/// Read one line from `buf` at `*pos` with the `read_line` semantics: `None`
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

/// One `io.capture` frame: stdout and stderr diverted while the block is live.
#[derive(Default)]
struct CaptureFrame {
	out: Vec<u8>,
	err: Vec<u8>,
}

/// One `io.with-stdin` frame: a fixed byte buffer the stdin reads draw from while
/// the block is live, with a running read cursor. Symmetric to `CaptureFrame` —
/// the reader's counterpart to capture's diverted writers.
struct StdinFrame {
	bytes: Vec<u8>,
	pos: usize,
}

struct HostState {
	io: Box<dyn HostIo>,
	/// The program's command-line arguments (interpreter + script path already
	/// stripped by the CLI), surfaced through the `io-args` import. Empty on the
	/// buffered test paths.
	args: Vec<String>,
	/// The `io.fail` abort message, stashed before the host traps so the runner can
	/// surface it as the program's `runtime error: <msg>` status.
	fail: Option<String>,
	/// The message the last failed `std/sys/io` call stashed (errno-style); returned
	/// by the `io-last-error` import, which `__io_result` queries on the err path.
	last_error: String,
	/// Bytes a read op produced that didn't fit the caller's first `dst` buffer; the
	/// wasm side then reserves the true size and drains this via `__io_copyout`. Empty
	/// on the common (fits-first-try) path. (The read overflow path.)
	read_stash: Vec<u8>,
	/// Active capture frames, used by the `io-capture-*` imports (`io.capture` in
	/// std/sys/io). While the stack is non-empty, `print`/`io.write` and the stderr
	/// writers land in the top frame's `out`/`err` instead of the real sinks. A stack
	/// so captures nest. Empty in normal runs.
	capture: Vec<CaptureFrame>,
	/// The stderr bytes of the most recently ended frame, parked between the paired
	/// `io-capture-end` (pops the frame, returns its stdout) and `io-capture-err`
	/// (returns this) calls that `io.capture` makes back-to-back.
	capture_err: Vec<u8>,
	/// Active `io.with-stdin` frames (the `io-with-stdin-*` imports). While the stack
	/// is non-empty, the stdin readers draw from the top frame instead of the real
	/// stdin sink, so a test can feed a program canned input. A stack so it nests;
	/// empty in normal runs.
	stdin_stack: Vec<StdinFrame>,
	/// `std/sys/net` runtime state: the socket table.
	net: HostNet,
	/// The shared readiness + completion reactor (poller + worker pool) that socket reads
	/// park on and offload clients (fs, db, …) submit blocking work to. (host/src/offload.rs.)
	reactor: Reactor,
	/// `std/sys/db` runtime state: the pinned SQLite worker (spawned on first use). Reports
	/// completions through `reactor`'s shared queue via a `CompletionSink`.
	db: HostDb,
	/// When this run is one shard of a parallel `pluma test`, its `(id, count)` —
	/// surfaced only through `io-env`'s reserved `PLUMA_TEST_SHARD` name so the test
	/// runner can select its slice, while real `io.args`/`io.env` stay untouched.
	pub shard: Option<(u32, u32)>,
}

impl HostState {
	/// Read one line of stdin: from the active `io.with-stdin` frame if any (draining
	/// it, then EOF), otherwise from the real stdin sink. The `io-read` counterpart to
	/// capture diverting the writers.
	fn read_line(&mut self) -> Option<String> {
		match self.stdin_stack.last_mut() {
			Some(frame) => read_line_from(&frame.bytes, &mut frame.pos),
			None => self.io.read_line(),
		}
	}

	/// Drain the rest of stdin: from the active `io.with-stdin` frame if any, otherwise
	/// the real sink. Backs `io-read-all`/`io-read-all-bytes`.
	fn read_rest(&mut self) -> Vec<u8> {
		match self.stdin_stack.last_mut() {
			Some(frame) => {
				let rest = frame.bytes[frame.pos..].to_vec();
				frame.pos = frame.bytes.len();
				rest
			}
			None => self.io.read_rest(),
		}
	}
}
