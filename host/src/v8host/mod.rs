// The V8 backend. Instantiates the WasmGC artifact under V8 — whose
// generational GC is what makes the boxed-value IR fast — driving the marshalled
// `pluma.*` imports as native V8 callbacks over the exported `"memory"` ArrayBuffer.
// Because the marshalling ABI makes every import scalar + scratch-memory bytes (no GC
// reflection), a stock JS engine can serve them at all; the import set reuses this
// crate's engine-independent core (`HostState`/`HostNet`/`NetRet`/`BufferedIo`/
// `read_line_from`).
//
// This is the deploy engine `cli` ships and `tests` snapshots against.
//
// Layout: this module holds the run drivers + the `pluma.*` registration table; the
// `marshal` submodule holds the shared V8↔scratch helpers; and one submodule per
// capability holds that capability's native import callbacks (`writers`, `fs`, `math`,
// `entropy`, `time`, `net`).

use std::sync::Once;

use crate::db::HostDb;
use crate::net::HostNet;
use crate::offload::Reactor;
use crate::{BufferedIo, CapturingIo, HostIo, HostState, RunCapture, RunResult, StdioIo};

mod compile;
mod compress;
mod crypto;
mod db;
mod entropy;
mod fs;
mod marshal;
mod math;
mod net;
mod offload;
mod regex;
mod time;
mod writers;

use marshal::{get_prop, read_mem, register};
// The native import callbacks, grouped by capability. Glob-imported so the registration
// table below can name each `cb_*` bare (the table is the canonical `pluma.*` surface).
use compile::cb_compile_wasm_hex;
use compress::*;
use crypto::*;
use db::*;
use entropy::*;
use fs::*;
use math::*;
use net::*;
use offload::*;
use regex::*;
use time::*;
use writers::*;

/// V8 platform init is process-global and one-shot.
static V8_INIT: Once = Once::new();

fn ensure_v8() {
	V8_INIT.call_once(|| {
		// Profiling hook: pass V8 flags (e.g. `--trace-gc`, semi-space sizing) without
		// recompiling. Unset in normal runs; set only when investigating GC/perf.
		if let Ok(flags) = std::env::var("PLUMA_V8_FLAGS") {
			v8::V8::set_flags_from_string(&flags);
		}
		let platform = v8::new_default_platform(0, false).make_shared();
		v8::V8::initialize_platform(platform);
		v8::V8::initialize();
	});
}

/// Everything a host-import callback needs, reached through the function's `External`
/// data pointer: the engine-independent host state plus the module's exported
/// `"memory"` (a `WebAssembly.Memory`, re-read each access so a `memory.grow` that
/// swapped the backing `ArrayBuffer` is picked up).
struct Ctx {
	state: HostState,
	memory: Option<v8::Global<v8::Object>>,
}

/// Compile + instantiate `bytes` under V8, run `_entry`, and report status + captured
/// stdout (stderr dropped). `stdin` feeds the buffered io sink.
pub fn run_wasm_v8(bytes: &[u8], stdin: &[u8]) -> RunResult {
	let cap = run_v8(bytes, Box::new(BufferedIo::new(stdin)), Vec::new());
	RunResult {
		status: cap.status,
		stdout: cap.stdout,
	}
}

/// Like `run_wasm_v8`, but captures stderr separately too — the snapshot suite
/// (`tests/run`) pins all three of status/stdout/stderr. `stdin` feeds the buffered
/// io sink.
pub fn run_wasm_v8_captured(bytes: &[u8], stdin: &[u8]) -> RunCapture {
	run_v8(bytes, Box::new(CapturingIo::new(stdin)), Vec::new())
}

/// Compile + instantiate `bytes` and run `_entry` once under V8, streaming
/// stdout/stderr to the process and reading stdin from it (the `cli`'s `pluma run`
/// path). `args` is the program's argv (`io.args`). Returns the process exit code; a
/// failure's message is already on stderr.
pub fn run_streaming_v8(bytes: &[u8], args: &[String]) -> i32 {
	let result = run_v8(bytes, Box::new(StdioIo::new()), args.to_vec());
	match result.status.as_str() {
		"ok" => 0,
		other => {
			let msg = other.strip_prefix("runtime error: ").unwrap_or(other);
			eprintln!("{msg}");
			1
		}
	}
}

/// Run a `pluma test` artifact and map the outcome to a process exit code.
///
/// Every test module runs in its OWN fresh V8 isolate — fresh wasm globals,
/// scheduler, and `HostState` — so no module can observe state another module
/// left behind; that isolation is the point. The artifact is compiled once and
/// its `CompiledWasmModule` shared (no recompilation) across the isolates, which
/// a bounded thread pool runs in parallel. `num_modules` is the suite count
/// (`program.test_suites.len()`); each isolate is told its index via the host's
/// reserved `PLUMA_TEST_SHARD` env name and runs just that module. Each isolate
/// captures its own report and prints it as soon as that module finishes, so
/// output streams in finish order (not module order) — a shared lock serializes
/// printing so modules never interleave mid-line. Once all threads join, one
/// aggregate summary closes the run. Exit code: 0 all-pass, 1 on any failure or
/// trap.
pub fn run_test_v8(bytes: &[u8], num_modules: usize, color: bool) -> i32 {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::{Arc, Mutex};

	ensure_v8();
	let num_items = num_modules;
	if num_items == 0 {
		return 0;
	}

	// Compile once on a throwaway isolate, then extract the shareable compiled
	// module. The native code outlives that isolate (it's held behind a shared
	// pointer), so every work item rebuilds its module object from it for free.
	let compiled = match compile_to_shared(bytes) {
		Some(c) => Arc::new(c),
		None => {
			eprintln!("wasm compile failed");
			return 1;
		}
	};

	// A bounded pool of `workers` threads pulls work items off a shared cursor.
	// Each item runs in its OWN fresh isolate, told its index via the reserved
	// `PLUMA_TEST_SHARD` host value, which the in-wasm runner (`run-all-sharded`)
	// turns into "run the suite at this index". The pool is sized to the machine.
	let workers = std::thread::available_parallelism()
		.map(|n| n.get())
		.unwrap_or(4)
		.clamp(1, num_items);
	let cursor = Arc::new(AtomicUsize::new(0));
	// Shared print state: the running [passed, failed, skipped, todo] totals plus
	// the aggregate exit code. The lock also serializes printing so a module's
	// report is emitted whole, never interleaved with another worker's output.
	let shared: Arc<Mutex<([i64; 4], i32)>> = Arc::new(Mutex::new(([0i64; 4], 0)));

	let handles: Vec<_> = (0..workers)
		.map(|_| {
			let compiled = Arc::clone(&compiled);
			let cursor = Arc::clone(&cursor);
			let shared = Arc::clone(&shared);
			std::thread::spawn(move || {
				loop {
					let i = cursor.fetch_add(1, Ordering::Relaxed);
					if i >= num_items {
						break;
					}
					let cap = run_in_fresh_isolate(
						ModuleSource::Compiled(&compiled),
						Box::new(CapturingIo::new(&[])),
						Vec::new(),
						Some((i as u32, num_items as u32)),
					);
					// Stream this module's report the moment it finishes. Each shard's
					// output ends with a `<RS>p f s t` counts line (see
					// `std/test.shard-counts-line`); pick those out and sum them into the
					// shared totals, printing the module tree without them. Holding the
					// lock across the whole print keeps modules from interleaving mid-line.
					let (totals, code) = &mut *shared.lock().unwrap();
					for line in cap.stdout.split_inclusive('\n') {
						match line.strip_prefix('\u{1e}') {
							Some(counts) => {
								for (slot, n) in totals.iter_mut().zip(
									counts
										.split_whitespace()
										.filter_map(|tok| tok.parse::<i64>().ok()),
								) {
									*slot += n;
								}
							}
							None => print!("{line}"),
						}
					}
					use std::io::Write;
					let _ = std::io::stdout().flush();
					eprint!("{}", cap.stderr);
					if test_exit_code(&cap.status) != 0 {
						*code = 1;
					}
				}
			})
		})
		.collect();

	let mut code = 0;
	for handle in handles {
		if handle.join().is_err() {
			eprintln!("a test worker thread panicked");
			code = 1;
		}
	}

	// All shards have printed their trees; close with one aggregate summary.
	let (totals, worker_code) = *shared.lock().unwrap();
	code |= worker_code;
	print_pool_summary(totals, color);
	if totals[1] > 0 {
		code = 1;
	}
	code
}

/// Print the one aggregate summary line for a pooled test run, mirroring
/// `std/test.summary-line`'s wording and color (bold green all-pass, bold red
/// otherwise) so a sharded run reads identically to a single-process one.
fn print_pool_summary(totals: [i64; 4], color: bool) {
	let [passed, failed, skipped, todo] = totals;
	let mut line = format!("{} of {} passed", passed, passed + failed);
	if skipped > 0 {
		line += &format!(", {skipped} skipped");
	}
	if todo > 0 {
		line += &format!(", {todo} todo");
	}
	println!();
	if color {
		let sgr = if failed == 0 { "1;32" } else { "1;31" };
		println!("\x1b[{sgr}m{line}\x1b[0m");
	} else {
		println!("{line}");
	}
}

/// Compile `bytes` to a `CompiledWasmModule` that can be shared across isolates.
/// The compiling isolate is dropped before returning; the compiled native code
/// survives behind V8's shared pointer.
fn compile_to_shared(bytes: &[u8]) -> Option<v8::CompiledWasmModule> {
	let isolate = &mut v8::Isolate::new(Default::default());
	let scope = &mut v8::HandleScope::new(isolate);
	let context = v8::Context::new(scope, Default::default());
	let scope = &mut v8::ContextScope::new(scope, context);
	let module = v8::WasmModuleObject::compile(scope, bytes)?;
	Some(module.get_compiled_module())
}

/// Map a run's status string to a `pluma test` exit code: `ok` → 0, a clean test
/// failure (`run-all` returns `err ""`) → 1 silently, and a genuine trap → 1 with
/// its message on stderr.
fn test_exit_code(status: &str) -> i32 {
	match status {
		"ok" => 0,
		"runtime error: " => 1,
		other => {
			let msg = other.strip_prefix("runtime error: ").unwrap_or(other);
			eprintln!("{msg}");
			1
		}
	}
}

/// Run `_entry` under V8 through the given io sink, returning status + captured
/// stdout/stderr (both empty for the streaming sink, stderr empty for the buffered
/// sink). `args` is the program's argv (`io-args`). The engine-neutral marshalling
/// core.
fn run_v8(bytes: &[u8], io: Box<dyn HostIo>, args: Vec<String>) -> RunCapture {
	ensure_v8();
	run_in_fresh_isolate(ModuleSource::Bytes(bytes), io, args, None)
}

/// A fresh `HostState` for one run — every per-run field at its empty default, the
/// io sink, argv, and optional test-shard supplied. Shared by the single-run and
/// per-shard drivers.
fn fresh_state(io: Box<dyn HostIo>, args: Vec<String>, shard: Option<(u32, u32)>) -> HostState {
	HostState {
		io,
		args,
		fail: None,
		last_error: String::new(),
		read_stash: Vec::new(),
		capture: Vec::new(),
		capture_err: Vec::new(),
		stdin_stack: Vec::new(),
		net: HostNet::default(),
		reactor: Reactor::default(),
		db: HostDb::default(),
		shard,
	}
}

/// Where `run_in_context` gets its module: freshly compiled from wire bytes, or
/// rebuilt — without recompiling — from a `CompiledWasmModule` shared across the
/// sharded test driver's isolates.
enum ModuleSource<'a> {
	Bytes(&'a [u8]),
	Compiled(&'a v8::CompiledWasmModule),
}

/// Build a fresh isolate + context, run `_entry` through it from the given module
/// source, and return its status + captured output. The isolate and all V8 handles
/// stay confined to this call (only the `Send` `CompiledWasmModule` ever crosses a
/// thread), so this is safe to call on a worker thread per shard.
fn run_in_fresh_isolate(
	src: ModuleSource,
	io: Box<dyn HostIo>,
	args: Vec<String>,
	shard: Option<(u32, u32)>,
) -> RunCapture {
	let mut ctx = Ctx {
		state: fresh_state(io, args, shard),
		memory: None,
	};
	let ctx_ptr = &mut ctx as *mut Ctx;

	let isolate = &mut v8::Isolate::new(Default::default());
	// Capture a stack trace for an uncaught exception (a wasm trap or `io.fail`), so
	// the trap arm in `run_in_context` can render a Pluma backtrace from the frames.
	isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 64);
	let scope = &mut v8::HandleScope::new(isolate);
	let context = v8::Context::new(scope, Default::default());
	let scope = &mut v8::ContextScope::new(scope, context);

	let status = run_in_context(scope, src, ctx_ptr);
	let stdout = ctx.state.io.captured_stdout();
	let stderr = ctx.state.io.captured_stderr();
	RunCapture {
		status,
		stdout,
		stderr,
	}
}

/// The body of a run, inside an entered context: compile the WasmGC module, then
/// instantiate it and run `_entry`. Returns the program status string.
fn run_in_context(scope: &mut v8::HandleScope, src: ModuleSource, ctx_ptr: *mut Ctx) -> String {
	// The trap source-map (module byte offset -> .pa line:col), read from the
	// artifact's `pluma_lines` section. Only the from-bytes path carries it; a shared
	// compiled module degrades to name-only frames.
	let line_table = match &src {
		ModuleSource::Bytes(bytes) => parse_line_table(bytes),
		ModuleSource::Compiled(_) => Vec::new(),
	};
	// Get the WasmGC module object — compile from bytes, or rebuild it from a
	// shared `CompiledWasmModule` (no recompilation; the native code is shared).
	let module = match src {
		ModuleSource::Bytes(bytes) => match v8::WasmModuleObject::compile(scope, bytes) {
			Some(m) => m,
			None => return "module error: compile failed".to_string(),
		},
		ModuleSource::Compiled(compiled) => {
			match v8::WasmModuleObject::from_compiled_module(scope, compiled) {
				Some(m) => m,
				None => return "module error: from_compiled_module failed".to_string(),
			}
		}
	};

	// Build the `{ pluma: { <imports> } }` import object. Each callback's `External`
	// data is the `Ctx` pointer it reads its state + memory through. The full set is
	// registered regardless of which subset a module declares (extras are ignored); a
	// callback must be a zero-sized fn item (not a fn pointer), so they're registered
	// one by one rather than from a table.
	let data: v8::Local<v8::Value> =
		v8::External::new(scope, ctx_ptr as *mut std::ffi::c_void).into();
	let pluma = v8::Object::new(scope);
	register(scope, pluma, data, "float_to_str", cb_float_to_str);
	// Writers — distinct zero-sized fn items per (stderr?, newline?) combination.
	register(scope, pluma, data, "print", cb_print);
	register(scope, pluma, data, "io-print", cb_print);
	register(scope, pluma, data, "io-print-err", cb_print_err);
	register(scope, pluma, data, "io-write", cb_write_out);
	register(scope, pluma, data, "io-write-err", cb_write_err);
	register(scope, pluma, data, "io-write-bytes", cb_write_out);
	register(scope, pluma, data, "io-write-err-bytes", cb_write_err);
	register(scope, pluma, data, "io-fail", cb_io_fail);
	// `io.capture` (std/sys/io): divert/collect stdout around a thunk (snapshot testing).
	register(scope, pluma, data, "io-capture-start", cb_capture_start);
	register(scope, pluma, data, "io-capture-out", cb_capture_out);
	register(scope, pluma, data, "io-capture-err", cb_capture_err);
	// `io.with-stdin` (std/sys/io): feed canned stdin to a thunk (the reader-side dual
	// of `io.capture`).
	register(
		scope,
		pluma,
		data,
		"io-with-stdin-start",
		cb_with_stdin_start,
	);
	register(scope, pluma, data, "io-with-stdin-end", cb_with_stdin_end);
	// std/sys/io reads / fs.
	register(scope, pluma, data, "io-read", cb_io_read);
	register(scope, pluma, data, "io-read-all", cb_io_read_all);
	register(
		scope,
		pluma,
		data,
		"io-read-all-bytes",
		cb_io_read_all_bytes,
	);
	register(scope, pluma, data, "io-read-file", cb_read_file);
	register(scope, pluma, data, "io-read-file-bytes", cb_read_file_bytes);
	register(scope, pluma, data, "io-read-dir", cb_read_dir);
	register(scope, pluma, data, "io-write-file", cb_write_file);
	register(scope, pluma, data, "io-write-file-bytes", cb_write_file);
	register(scope, pluma, data, "io-append-file", cb_append_file);
	register(scope, pluma, data, "io-append-file-bytes", cb_append_file);
	register(scope, pluma, data, "io-delete-file", cb_delete_file);
	register(scope, pluma, data, "io-make-dir", cb_make_dir);
	register(scope, pluma, data, "io-file-exists", cb_file_exists);
	register(scope, pluma, data, "io-is-dir", cb_is_dir);
	register(scope, pluma, data, "io-last-error", cb_last_error);
	register(scope, pluma, data, "io-copyout", cb_io_copyout);
	// std/sys/compile — the playground compile primitive (source string -> wasm hex),
	// invoking this workspace's own compiler/ir/wasm pipeline.
	register(scope, pluma, data, "compile-wasm-hex", cb_compile_wasm_hex);
	// std/sys/io process surface (Process capability) — argv, env, exit.
	register(scope, pluma, data, "io-args", cb_io_args);
	register(scope, pluma, data, "io-env", cb_io_env);
	register(scope, pluma, data, "io-exit", cb_io_exit);
	register(scope, pluma, data, "io-cwd", cb_io_cwd);
	// Unary float math — the libm calls (`(f64) -> f64`).
	register(scope, pluma, data, "math-log", cb_math_log);
	register(scope, pluma, data, "math-log10", cb_math_log10);
	register(scope, pluma, data, "math-log2", cb_math_log2);
	register(scope, pluma, data, "math-exp", cb_math_exp);
	register(scope, pluma, data, "math-sin", cb_math_sin);
	register(scope, pluma, data, "math-cos", cb_math_cos);
	// std/random / std/uuid (Entropy).
	register(scope, pluma, data, "random-int", cb_random_int);
	register(scope, pluma, data, "random-float", cb_random_float);
	register(scope, pluma, data, "random-int-range", cb_random_int_range);
	register(scope, pluma, data, "random-bytes", cb_random_bytes);
	register(scope, pluma, data, "uuid-v4", cb_uuid_v4);
	register(scope, pluma, data, "uuid-v7", cb_uuid_v7);
	register(scope, pluma, data, "uuid-parse", cb_uuid_parse);
	// std/regex (V8's RegExp).
	register(scope, pluma, data, "regex-find-all", cb_regex_find_all);
	// std/time clock surface (Clock capability) — wall/monotonic clock, sleep, parse.
	register(scope, pluma, data, "time-now", cb_time_now);
	register(scope, pluma, data, "time-monotonic", cb_time_monotonic);
	register(scope, pluma, data, "time-sleep", cb_time_sleep);
	register(scope, pluma, data, "time-parse", cb_time_parse);
	// std/sys/net — socket ops (the multi-result ones return a `[status, n]` JS array).
	register(scope, pluma, data, "net-listen", cb_net_listen);
	register(scope, pluma, data, "net-listen-tls", cb_net_listen_tls);
	register(scope, pluma, data, "net-connect", cb_net_connect);
	register(scope, pluma, data, "net-connect-tls", cb_net_connect_tls);
	register(scope, pluma, data, "net-close", cb_net_close);
	register(scope, pluma, data, "net-local-addr", cb_net_local_addr);
	register(scope, pluma, data, "net-accept", cb_net_accept);
	register(scope, pluma, data, "net-read", cb_net_read);
	register(scope, pluma, data, "net-write", cb_net_write);
	// Shared offload reactor controls (host/src/offload.rs): the block step + reap, driven by the
	// in-wasm scheduler for net *and* every offload client (fs, db, …). `io-poll` blocks the
	// thread synchronously (fine in a V8 callback) until a parked socket is ready or a worker
	// completion lands. Plus the v0 `offload-sleep` proving op (sleep on a pool worker).
	register(scope, pluma, data, "io-poll", cb_io_poll);
	register(scope, pluma, data, "io-unwatch", cb_io_unwatch);
	register(scope, pluma, data, "offload-sleep", cb_offload_sleep);
	// std/sys/fs (host/src/offload.rs): one generic op-code dispatch — `fs-op` runs the op on a
	// pool worker (async, the default surface), `fs-op-sync` runs it inline (the `-sync`
	// twin). Both shape `(dst, cap) -> bytes` like the other reads.
	register(scope, pluma, data, "fs-op", cb_fs_op);
	register(scope, pluma, data, "fs-op-sync", cb_fs_op_sync);
	// std/sys/db (host/src/db.rs): one generic `db-op` (open/execute/close by op-code),
	// offloaded to the pinned SQLite worker — async only, no `-sync` twin.
	register(scope, pluma, data, "db-op", cb_db_op);
	// std/compress (host/src/compress.rs): gzip/brotli byte transforms, riding the
	// `io.read-file-bytes` marshalling shape (bytes in scratch, bytes out via `(dst, cap)`).
	register(scope, pluma, data, "gzip-encode", cb_gzip_encode);
	register(scope, pluma, data, "gzip-decode", cb_gzip_decode);
	register(scope, pluma, data, "brotli-encode", cb_brotli_encode);
	register(scope, pluma, data, "brotli-decode", cb_brotli_decode);
	// SHA-1 (host/src/crypto.rs): the WebSocket handshake accept-key digest, same ABI.
	register(scope, pluma, data, "sha1", cb_sha1);
	// std/web/fetch — the browser HTTP transport, here a blocking HTTP/1.1 exchange.
	register(scope, pluma, data, "web-fetch", cb_web_fetch);
	// std/event — SSR stubs (a server build constructs view handlers but never runs
	// them; these link the import and are never actually called).
	register(
		scope,
		pluma,
		data,
		"event-target-value",
		cb_event_target_value,
	);
	register(
		scope,
		pluma,
		data,
		"event-target-checked",
		cb_event_target_checked,
	);
	register(
		scope,
		pluma,
		data,
		"event-prevent-default",
		cb_event_prevent_default,
	);
	register(scope, pluma, data, "dom-child-at", cb_dom_child_at);
	let imports = v8::Object::new(scope);
	let pluma_key = v8::String::new(scope, "pluma").unwrap();
	imports.set(scope, pluma_key.into(), pluma.into());

	// `new WebAssembly.Instance(module, imports)`.
	let instance = match instantiate(scope, module, imports) {
		Ok(i) => i,
		Err(e) => return e,
	};
	let exports = get_prop(scope, instance, "exports")
		.and_then(|v| v.to_object(scope))
		.expect("instance.exports");

	// Stash the exported memory so the import callbacks can reach it.
	let memory = get_prop(scope, exports, "memory")
		.and_then(|v| v.to_object(scope))
		.expect("memory export");
	unsafe { &mut *ctx_ptr }.memory = Some(v8::Global::new(scope, memory));

	let entry: v8::Local<v8::Function> = get_prop(scope, exports, "_entry")
		.and_then(|v| v.try_into().ok())
		.expect("_entry export");

	// Call `_entry(null)`, catching an `io.fail` (or any) trap.
	let recv = v8::undefined(scope).into();
	let null = v8::null(scope).into();
	let tc = &mut v8::TryCatch::new(scope);
	let ret = entry.call(tc, recv, &[null]);
	match ret {
		Some(ret) => {
			// Ok-path: probe the return for a `result.err` via `__entry_error`.
			entry_error(tc, exports, ret)
		}
		None => {
			// A trap. An `io.fail` stashed its message host-side; surface that, else the
			// raw V8 exception text (e.g. a wasm RuntimeError) so the reason isn't lost.
			let base = match unsafe { &*ctx_ptr }.state.fail.clone() {
				Some(msg) => format!("runtime error: {msg}"),
				None => {
					let detail = tc
						.exception()
						.map(|e| e.to_rust_string_lossy(tc))
						.unwrap_or_default();
					if detail.is_empty() {
						"runtime error: trap".to_string()
					} else {
						format!("runtime error: {detail}")
					}
				}
			};
			// An optional escape hatch for calibrating against V8's own rendering.
			if std::env::var_os("PLUMA_TRAP_STACK").is_some() {
				if let Some(st) = tc.stack_trace() {
					eprintln!("[trap stack] {}", st.to_rust_string_lossy(tc));
				}
			}
			// Append a Pluma backtrace: the named wasm frames (innermost first) that the
			// `name` section labelled with each function's `module.name`. V8 prefixes a
			// wasm function name with `$`; strip it. Synthetic runtime frames (the entry
			// bootstrap, scheduler helpers, builtin wrappers — all named with a leading
			// `__`) are scaffolding, not user code, so they're skipped.
			let mut out = base;
			if let Some(msg) = tc.message() {
				if let Some(st) = msg.get_stack_trace(tc) {
					for i in 0..st.get_frame_count() {
						let Some(frame) = st.get_frame(tc, i) else {
							continue;
						};
						let Some(name) = frame.get_function_name(tc) else {
							continue;
						};
						let name = name.to_rust_string_lossy(tc);
						let name = name.strip_prefix('$').unwrap_or(&name);
						if name.is_empty() || name.starts_with("__") {
							continue;
						}
						out.push_str("\n  at ");
						out.push_str(name);
						// V8 reports a wasm frame's module byte offset as its 1-based
						// column; resolve it to the trap's .pa line:col. The label is the
						// module, so this reads as a `module:line:col` source location
						// (rendered 1-based). Without a mapping, the bare module stands.
						let col = frame.get_column();
						if col > 0 {
							if let Some((line, c)) = lookup_line(&line_table, (col - 1) as u32) {
								out.push_str(&format!(":{}:{}", line + 1, c + 1));
							}
						}
					}
				}
			}
			out
		}
	}
}

/// Decode the `pluma_lines` custom section into an offset-sorted line table of
/// `(module byte offset, line, col)` at statement boundaries (all 0-based). Returns
/// empty if the section is absent or malformed.
fn parse_line_table(bytes: &[u8]) -> Vec<(u32, u32, u32)> {
	for payload in wasmparser::Parser::new(0).parse_all(bytes).flatten() {
		if let wasmparser::Payload::CustomSection(cs) = payload {
			if cs.name() == "pluma_lines" {
				return decode_line_table(cs.data());
			}
		}
	}
	Vec::new()
}

fn decode_line_table(data: &[u8]) -> Vec<(u32, u32, u32)> {
	let mut out = Vec::new();
	if data.len() < 4 {
		return out;
	}
	let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
	let mut p = 4;
	for _ in 0..count {
		if p + 12 > data.len() {
			break;
		}
		let rd = |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
		out.push((rd(p), rd(p + 4), rd(p + 8)));
		p += 12;
	}
	out
}

/// The statement containing `off` is the one with the greatest start offset not
/// past it. Returns its `(line, col)`, or `None` if `off` precedes the first mark.
fn lookup_line(table: &[(u32, u32, u32)], off: u32) -> Option<(u32, u32)> {
	let i = table.partition_point(|&(o, _, _)| o <= off);
	(i > 0).then(|| {
		let (_, line, col) = table[i - 1];
		(line, col)
	})
}

/// `new WebAssembly.Instance(module, imports)`.
fn instantiate<'s>(
	scope: &mut v8::HandleScope<'s>,
	module: v8::Local<'s, v8::WasmModuleObject>,
	imports: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, String> {
	let global = scope.get_current_context().global(scope);
	let wasm = get_prop(scope, global, "WebAssembly")
		.and_then(|v| v.to_object(scope))
		.ok_or("no WebAssembly global")?;
	let ctor: v8::Local<v8::Function> = get_prop(scope, wasm, "Instance")
		.and_then(|v| v.try_into().ok())
		.ok_or("no WebAssembly.Instance")?;
	let tc = &mut v8::TryCatch::new(scope);
	match ctor.new_instance(tc, &[module.into(), imports.into()]) {
		Some(i) => Ok(i),
		None => {
			let msg = tc
				.exception()
				.map(|e| e.to_rust_string_lossy(tc))
				.unwrap_or_default();
			Err(format!("module error: instantiate failed: {msg}"))
		}
	}
}

/// Call `__entry_error(ret) -> i32` and read the message out of scratch on a non-
/// negative length (a `result.err` `main` returned), else `ok`.
fn entry_error(
	scope: &mut v8::HandleScope,
	exports: v8::Local<v8::Object>,
	ret: v8::Local<v8::Value>,
) -> String {
	let f: v8::Local<v8::Function> =
		match get_prop(scope, exports, "__entry_error").and_then(|v| v.try_into().ok()) {
			Some(f) => f,
			None => return "ok".to_string(),
		};
	let recv = v8::undefined(scope).into();
	let len = f
		.call(scope, recv, &[ret])
		.and_then(|v| v.int32_value(scope))
		.unwrap_or(-1);
	if len < 0 {
		return "ok".to_string();
	}
	// The message is at scratch offset 0 (where `__send_bytes` writes).
	let memory = get_prop(scope, exports, "memory")
		.and_then(|v| v.to_object(scope))
		.expect("memory export");
	let bytes = read_mem(scope, memory, 0, len as usize);
	format!("runtime error: {}", String::from_utf8_lossy(&bytes))
}
