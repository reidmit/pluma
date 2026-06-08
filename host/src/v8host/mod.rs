// The V8 backend (ABI.md Phase 2). Instantiates the WasmGC artifact under V8 — whose
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

use crate::net::HostNet;
use crate::{BufferedIo, CapturingIo, HostIo, HostState, RunCapture, RunResult, StdioIo};

mod entropy;
mod fs;
mod marshal;
mod math;
mod net;
mod time;
mod writers;

use marshal::{get_prop, read_mem, register};
// The native import callbacks, grouped by capability. Glob-imported so the registration
// table below can name each `cb_*` bare (the table is the canonical `pluma.*` surface).
use entropy::*;
use fs::*;
use math::*;
use net::*;
use time::*;
use writers::*;

/// V8 platform init is process-global and one-shot.
static V8_INIT: Once = Once::new();

fn ensure_v8() {
	V8_INIT.call_once(|| {
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

/// Run a `pluma test` artifact under V8, streaming the report to stdout, and map
/// the outcome to a process exit code. The runner (`std.test.run-all`) prints
/// everything itself and returns `ok ()` on success or `err ""` on test failures
/// — so a clean failure (`"runtime error: "` with an empty message) just exits 1
/// silently, while a genuine trap (a crashing case) still surfaces its message.
pub fn run_test_v8(bytes: &[u8]) -> i32 {
	let result = run_v8(bytes, Box::new(StdioIo::new()), Vec::new());
	match result.status.as_str() {
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

	let mut ctx = Ctx {
		state: HostState {
			io,
			args,
			fail: None,
			last_error: String::new(),
			read_stash: Vec::new(),
			net: HostNet::default(),
		},
		memory: None,
	};
	let ctx_ptr = &mut ctx as *mut Ctx;

	let isolate = &mut v8::Isolate::new(Default::default());
	let scope = &mut v8::HandleScope::new(isolate);
	let context = v8::Context::new(scope, Default::default());
	let scope = &mut v8::ContextScope::new(scope, context);

	let status = run_in_context(scope, bytes, ctx_ptr);
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
fn run_in_context(scope: &mut v8::HandleScope, bytes: &[u8], ctx_ptr: *mut Ctx) -> String {
	// Compile the WasmGC module.
	let module = match v8::WasmModuleObject::compile(scope, bytes) {
		Some(m) => m,
		None => return "module error: compile failed".to_string(),
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
	// std.sys.io reads / fs.
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
	// std.sys.io process surface (Process capability) — argv, env, exit.
	register(scope, pluma, data, "io-args", cb_io_args);
	register(scope, pluma, data, "io-env", cb_io_env);
	register(scope, pluma, data, "io-exit", cb_io_exit);
	// Unary float math — the libm calls (`(f64) -> f64`).
	register(scope, pluma, data, "math-log", cb_math_log);
	register(scope, pluma, data, "math-log10", cb_math_log10);
	register(scope, pluma, data, "math-log2", cb_math_log2);
	register(scope, pluma, data, "math-exp", cb_math_exp);
	register(scope, pluma, data, "math-sin", cb_math_sin);
	register(scope, pluma, data, "math-cos", cb_math_cos);
	// std.random / std.uuid (Entropy).
	register(scope, pluma, data, "random-int", cb_random_int);
	register(scope, pluma, data, "random-float", cb_random_float);
	register(scope, pluma, data, "random-int-range", cb_random_int_range);
	register(scope, pluma, data, "random-bytes", cb_random_bytes);
	register(scope, pluma, data, "uuid-v4", cb_uuid_v4);
	register(scope, pluma, data, "uuid-v7", cb_uuid_v7);
	register(scope, pluma, data, "uuid-parse", cb_uuid_parse);
	// std.time clock surface (Clock capability) — wall/monotonic clock, sleep, parse.
	register(scope, pluma, data, "time-now", cb_time_now);
	register(scope, pluma, data, "time-monotonic", cb_time_monotonic);
	register(scope, pluma, data, "time-sleep", cb_time_sleep);
	register(scope, pluma, data, "time-parse", cb_time_parse);
	// std.sys.net — socket ops (the multi-result ones return a `[status, n]` JS array) +
	// the reactor controls. `net-poll` blocks the thread synchronously (fine in a V8
	// callback) until a parked socket is ready — the reactor step.
	register(scope, pluma, data, "net-listen", cb_net_listen);
	register(scope, pluma, data, "net-connect", cb_net_connect);
	register(scope, pluma, data, "net-close", cb_net_close);
	register(scope, pluma, data, "net-local-addr", cb_net_local_addr);
	register(scope, pluma, data, "net-accept", cb_net_accept);
	register(scope, pluma, data, "net-read", cb_net_read);
	register(scope, pluma, data, "net-write", cb_net_write);
	register(scope, pluma, data, "net-poll", cb_net_poll);
	register(scope, pluma, data, "net-unwatch", cb_net_unwatch);
	// std.web.fetch — the browser HTTP transport, here a blocking HTTP/1.1 exchange.
	register(scope, pluma, data, "web-fetch", cb_web_fetch);
	// std.event — SSR stubs (a server build constructs view handlers but never runs
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
			// raw exception text.
			let _ = tc.exception();
			match unsafe { &*ctx_ptr }.state.fail.clone() {
				Some(msg) => format!("runtime error: {msg}"),
				None => "runtime error: trap".to_string(),
			}
		}
	}
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
