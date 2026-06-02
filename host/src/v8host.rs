// The V8 backend (ABI.md Phase 2). Instantiates the WasmGC artifact under V8 — whose
// generational GC is what makes the boxed-value IR fast — driving the marshalled
// `pluma.*` imports as native V8 callbacks over the exported `"memory"` ArrayBuffer.
// Because the marshalling ABI makes every import scalar + scratch-memory bytes (no GC
// reflection), a stock JS engine can serve them at all; the import set reuses this
// crate's engine-independent core (`HostState`/`HostNet`/`NetRet`/`BufferedIo`/
// `read_line_from`).
//
// This is the deploy engine `cli` ships and `conformance` diffs against the VM oracle.

use std::sync::Once;

use crate::{BufferedIo, HostIo, HostNet, HostState, NetRet, RunResult, StdioIo};

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
/// stdout (the conformance differential shape, mirroring `run_wasm`). `stdin` feeds the
/// buffered io sink.
pub fn run_wasm_v8(bytes: &[u8], stdin: &[u8]) -> RunResult {
	run_v8(bytes, Box::new(BufferedIo::new(stdin)))
}

/// Compile + instantiate `bytes` and run `_entry` once under V8, streaming
/// stdout/stderr to the process and reading stdin from it (the `cli`'s `pluma run`
/// path). Returns the process exit code; a failure's message is already on stderr.
pub fn run_streaming_v8(bytes: &[u8]) -> i32 {
	let result = run_v8(bytes, Box::new(StdioIo::new()));
	match result.status.as_str() {
		"ok" => 0,
		other => {
			let msg = other.strip_prefix("runtime error: ").unwrap_or(other);
			eprintln!("{msg}");
			1
		}
	}
}

/// Run `_entry` under V8 through the given io sink, returning status + captured stdout
/// (empty for the streaming sink). The engine-neutral marshalling core.
fn run_v8(bytes: &[u8], io: Box<dyn HostIo>) -> RunResult {
	ensure_v8();

	let mut ctx = Ctx {
		state: HostState {
			io,
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
	RunResult { status, stdout }
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
	// core.io reads / fs.
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
	// Unary float math — the libm calls (`(f64) -> f64`), same as the VM.
	register(scope, pluma, data, "math-log", cb_math_log);
	register(scope, pluma, data, "math-log10", cb_math_log10);
	register(scope, pluma, data, "math-log2", cb_math_log2);
	register(scope, pluma, data, "math-exp", cb_math_exp);
	register(scope, pluma, data, "math-sin", cb_math_sin);
	register(scope, pluma, data, "math-cos", cb_math_cos);
	// core.net — socket ops (the multi-result ones return a `[status, n]` JS array) +
	// the reactor controls. `net-poll` blocks the thread synchronously (fine in a V8
	// callback) until a parked socket is ready, mirroring the VM's reactor step.
	register(scope, pluma, data, "net-listen", cb_net_listen);
	register(scope, pluma, data, "net-connect", cb_net_connect);
	register(scope, pluma, data, "net-close", cb_net_close);
	register(scope, pluma, data, "net-local-addr", cb_net_local_addr);
	register(scope, pluma, data, "net-accept", cb_net_accept);
	register(scope, pluma, data, "net-read", cb_net_read);
	register(scope, pluma, data, "net-write", cb_net_write);
	register(scope, pluma, data, "net-poll", cb_net_poll);
	register(scope, pluma, data, "net-unwatch", cb_net_unwatch);
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

// --------------------------------------------------------------------------
// Small V8 helpers.
// --------------------------------------------------------------------------

/// `obj.<key>` as a `Local<Value>`.
fn get_prop<'s>(
	scope: &mut v8::HandleScope<'s>,
	obj: v8::Local<v8::Object>,
	key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
	let k = v8::String::new(scope, key)?;
	obj.get(scope, k.into())
}

/// The exported memory's current backing-store data pointer + length (re-read each
/// time: `memory.grow` swaps the `ArrayBuffer`).
fn mem_slice<'s>(
	scope: &mut v8::HandleScope<'s>,
	memory: v8::Local<v8::Object>,
) -> (*mut u8, usize) {
	let buffer: v8::Local<v8::ArrayBuffer> = get_prop(scope, memory, "buffer")
		.and_then(|v| v.try_into().ok())
		.expect("memory.buffer");
	let store = buffer.get_backing_store();
	let len = store.byte_length();
	let ptr = match store.data() {
		Some(p) => p.as_ptr() as *mut u8,
		None => std::ptr::null_mut(),
	};
	(ptr, len)
}

/// Read `len` bytes of the wasm memory at `off`.
fn read_mem(
	scope: &mut v8::HandleScope,
	memory: v8::Local<v8::Object>,
	off: usize,
	len: usize,
) -> Vec<u8> {
	let (ptr, cap) = mem_slice(scope, memory);
	if ptr.is_null() || off + len > cap {
		return Vec::new();
	}
	unsafe { std::slice::from_raw_parts(ptr.add(off), len).to_vec() }
}

/// Write `data` into the wasm memory at `off`.
fn write_mem(scope: &mut v8::HandleScope, memory: v8::Local<v8::Object>, off: usize, data: &[u8]) {
	let (ptr, cap) = mem_slice(scope, memory);
	if ptr.is_null() || off + data.len() > cap {
		return;
	}
	unsafe { std::slice::from_raw_parts_mut(ptr.add(off), data.len()).copy_from_slice(data) }
}

/// Recover the `Ctx` from a callback's `External` data, plus a `Local` of the exported
/// memory opened in the callback's scope.
fn ctx_and_mem<'s>(
	scope: &mut v8::HandleScope<'s>,
	args: &v8::FunctionCallbackArguments,
) -> (&'s mut Ctx, v8::Local<'s, v8::Object>) {
	let ext = v8::Local::<v8::External>::try_from(args.data()).expect("callback External data");
	let ctx = unsafe { &mut *(ext.value() as *mut Ctx) };
	let mem = ctx.memory.as_ref().expect("memory set before callbacks");
	let mem = v8::Local::new(scope, mem);
	(ctx, mem)
}

/// An `i32` callback argument.
fn argi(scope: &mut v8::HandleScope, args: &v8::FunctionCallbackArguments, i: i32) -> i32 {
	args.get(i).int32_value(scope).unwrap_or(0)
}

// --------------------------------------------------------------------------
// The host imports, as native V8 callbacks. (First cut: the writers + float_to_str;
// io/net follow.)
// --------------------------------------------------------------------------

/// Install one `pluma.<name>` native import on `pluma`, wired to the shared `Ctx` via
/// `data`. Generic over the callback because `MapFnTo` requires the zero-sized fn item
/// (a fn pointer would have nonzero size and fail its const check).
fn register<'s>(
	scope: &mut v8::HandleScope<'s>,
	pluma: v8::Local<'s, v8::Object>,
	data: v8::Local<'s, v8::Value>,
	name: &str,
	cb: impl v8::MapFnTo<v8::FunctionCallback>,
) {
	let key = v8::String::new(scope, name).unwrap();
	let f = v8::Function::builder(cb).data(data).build(scope).unwrap();
	pluma.set(scope, key.into(), f.into());
}

/// The shared writer body: read the pre-rendered `(ptr, len)` bytes out of scratch and
/// write them to stdout/stderr, optionally newline-terminated. wasm already rendered
/// (via `__tostring` or the raw `$bytes`), so the host just shuttles bytes.
fn write_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	to_err: bool,
	newline: bool,
) {
	let (ptr, len) = (argi(scope, args, 0), argi(scope, args, 1));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let mut bytes = read_mem(scope, mem, ptr.max(0) as usize, len.max(0) as usize);
	if newline {
		bytes.push(b'\n');
	}
	if to_err {
		ctx.state.io.write_err(&bytes);
	} else {
		ctx.state.io.write_out(&bytes);
	}
}

fn cb_print(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, _r: v8::ReturnValue) {
	write_impl(s, &a, false, true);
}
fn cb_print_err(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, _r: v8::ReturnValue) {
	write_impl(s, &a, true, true);
}
fn cb_write_out(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, _r: v8::ReturnValue) {
	write_impl(s, &a, false, false);
}
fn cb_write_err(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, _r: v8::ReturnValue) {
	write_impl(s, &a, true, false);
}

/// `io-fail(ptr, len)`: stash the pre-rendered message host-side, then throw — the
/// `_entry` call unwinds, and the runner surfaces the stashed message (mirroring the
/// VM's abort).
fn cb_io_fail(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let (ptr, len) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = read_mem(scope, mem, ptr.max(0) as usize, len.max(0) as usize);
	ctx.state.fail = Some(String::from_utf8_lossy(&bytes).into_owned());
	let exc = v8::String::new(scope, "io.fail").unwrap();
	scope.throw_exception(exc.into());
}

// --------------------------------------------------------------------------
// core.io reads / fs. Each callback reads path/data out of scratch, runs the
// `std::fs`/stdin op, delivers bytes back into the caller's `(dst,cap)` buffer
// (overflow → `read_stash` for `io-copyout`), and sets `last_error` on failure.
// --------------------------------------------------------------------------

/// A UTF-8-lossy string read of `(ptr, len)` scratch bytes.
fn read_str(scope: &mut v8::HandleScope, mem: v8::Local<v8::Object>, ptr: i32, len: i32) -> String {
	let b = read_mem(scope, mem, ptr.max(0) as usize, len.max(0) as usize);
	String::from_utf8_lossy(&b).into_owned()
}

/// Deliver a read's `bytes` to the caller's `(dst, cap)` buffer (V8 analogue of
/// `deliver_read`): write into scratch if they fit, else stash for `io-copyout`;
/// return the true length.
fn deliver_read_v8(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ctx: &mut Ctx,
	dst: i32,
	cap: i32,
	bytes: Vec<u8>,
) -> i32 {
	let len = bytes.len();
	if len <= cap.max(0) as usize {
		write_mem(scope, mem, dst.max(0) as usize, &bytes);
	} else {
		ctx.state.read_stash = bytes;
	}
	len as i32
}

fn cb_io_read(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let line = ctx.state.io.read_line();
	let n = match line {
		Some(l) => deliver_read_v8(scope, mem, ctx, dst, cap, l.into_bytes()),
		None => {
			ctx.state.last_error = "EOF".to_string();
			-1
		}
	};
	rv.set_int32(n);
}

fn cb_io_read_all(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = ctx.state.io.read_rest();
	let s = String::from_utf8_lossy(&bytes).into_owned();
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, s.into_bytes()));
}

fn cb_io_read_all_bytes(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = ctx.state.io.read_rest();
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, bytes));
}

fn read_file_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	as_bytes: bool,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (dst, cap) = (argi(scope, args, 2), argi(scope, args, 3));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let res = if as_bytes {
		std::fs::read(&path)
	} else {
		std::fs::read_to_string(&path).map(String::into_bytes)
	};
	let n = match res {
		Ok(b) => deliver_read_v8(scope, mem, ctx, dst, cap, b),
		Err(e) => {
			ctx.state.last_error = e.to_string();
			-1
		}
	};
	rv.set_int32(n);
}
fn cb_read_file(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	read_file_impl(s, &a, false, &mut r);
}
fn cb_read_file_bytes(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	read_file_impl(s, &a, true, &mut r);
}

fn cb_read_dir(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let path = read_str(scope, mem, pp, pl);
	let n = match std::fs::read_dir(&path) {
		Ok(entries) => {
			let mut names: Vec<String> = Vec::new();
			let mut err: Option<String> = None;
			for e in entries {
				match e {
					Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
					Err(e) => {
						err = Some(e.to_string());
						break;
					}
				}
			}
			match err {
				Some(msg) => {
					ctx.state.last_error = msg;
					-1
				}
				None => {
					names.sort();
					let mut blob = Vec::new();
					for nm in &names {
						blob.extend_from_slice(nm.as_bytes());
						blob.push(0);
					}
					deliver_read_v8(scope, mem, ctx, dst, cap, blob)
				}
			}
		}
		Err(e) => {
			ctx.state.last_error = e.to_string();
			-1
		}
	};
	rv.set_int32(n);
}

fn write_file_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	append: bool,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (dp, dl) = (argi(scope, args, 2), argi(scope, args, 3));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let data = read_mem(scope, mem, dp.max(0) as usize, dl.max(0) as usize);
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
	rv.set_int32(io_status(ctx, res));
}
fn cb_write_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	write_file_impl(s, &a, false, &mut r);
}
fn cb_append_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	write_file_impl(s, &a, true, &mut r);
}

fn path_op_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	op: impl FnOnce(&str) -> std::io::Result<()>,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let res = op(&path);
	rv.set_int32(io_status(ctx, res));
}
fn cb_delete_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	path_op_impl(s, &a, |p| std::fs::remove_file(p), &mut r);
}
fn cb_make_dir(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	path_op_impl(s, &a, |p| std::fs::create_dir_all(p), &mut r);
}

fn query_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	is_dir: bool,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (_ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let p = std::path::Path::new(&path);
	let b = if is_dir { p.is_dir() } else { p.exists() };
	rv.set_int32(b as i32);
}
fn cb_file_exists(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	query_impl(s, &a, false, &mut r);
}
fn cb_is_dir(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	query_impl(s, &a, true, &mut r);
}

/// `io-last-error(dst, cap) -> len`: write the stashed message into scratch (truncated
/// to `cap`); errno strings are short, so no overflow stash.
fn cb_last_error(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let msg = ctx.state.last_error.clone();
	let bytes = msg.as_bytes();
	let len = bytes.len().min(cap.max(0) as usize);
	write_mem(scope, mem, dst.max(0) as usize, &bytes[..len]);
	rv.set_int32(len as i32);
}

/// `io-copyout(dst)`: drain the read stash into scratch at `dst` (the overflow path).
fn cb_io_copyout(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let dst = argi(scope, &args, 0);
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let stash = std::mem::take(&mut ctx.state.read_stash);
	write_mem(scope, mem, dst.max(0) as usize, &stash);
}

/// Shape an fs `Result<()>` into a `(0 ok / 2 err)` status, stashing the errno text.
fn io_status(ctx: &mut Ctx, res: std::io::Result<()>) -> i32 {
	match res {
		Ok(()) => 0,
		Err(e) => {
			ctx.state.last_error = e.to_string();
			2
		}
	}
}

/// A unary `(f64) -> f64` math import: apply `f` to arg 0.
fn math_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	rv: &mut v8::ReturnValue,
	f: fn(f64) -> f64,
) {
	let x = args.get(0).number_value(scope).unwrap_or(0.0);
	rv.set_double(f(x));
}
fn cb_math_log(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	math_impl(s, &a, &mut r, f64::ln);
}
fn cb_math_log10(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	math_impl(s, &a, &mut r, f64::log10);
}
fn cb_math_log2(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	math_impl(s, &a, &mut r, f64::log2);
}
fn cb_math_exp(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	math_impl(s, &a, &mut r, f64::exp);
}
fn cb_math_sin(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	math_impl(s, &a, &mut r, f64::sin);
}
fn cb_math_cos(s: &mut v8::HandleScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue) {
	math_impl(s, &a, &mut r, f64::cos);
}

// --------------------------------------------------------------------------
// core.net. Reuses `HostNet`/`NetRet`; the multi-result ops return a `[status, n]`
// JS array (how V8 surfaces a multi-value wasm import result).
// --------------------------------------------------------------------------

/// Set a multi-result return as a `[a, b]` JS array.
fn set_pair(scope: &mut v8::HandleScope, rv: &mut v8::ReturnValue, a: i32, b: i32) {
	let arr = v8::Array::new(scope, 2);
	let av: v8::Local<v8::Value> = v8::Integer::new(scope, a).into();
	arr.set_index(scope, 0, av);
	let bv: v8::Local<v8::Value> = v8::Integer::new(scope, b).into();
	arr.set_index(scope, 1, bv);
	rv.set(arr.into());
}

/// Shape a scalar `NetRet` (id / count / nothing) into `(status, n)`; an error stashes
/// its message in `last_error` (read back via `io-last-error`, like core.io).
fn net_scalar_v8(ctx: &mut Ctx, ret: NetRet) -> (i32, i32) {
	match ret {
		NetRet::OkInt(v) => (0, v),
		NetRet::OkNothing => (0, 0),
		NetRet::WouldBlock => (1, 0),
		NetRet::Err(e) => {
			ctx.state.last_error = e;
			(2, 0)
		}
		NetRet::OkBytes(_) | NetRet::OkStr(_) => unreachable!("net_scalar_v8 on a byte op"),
	}
}

/// Shape a byte-returning `NetRet` (read bytes / local-addr string) into `(status,
/// len)`, writing the payload into scratch at `dst` (truncated to `cap`).
fn net_bytes_v8(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ctx: &mut Ctx,
	dst: i32,
	cap: i32,
	ret: NetRet,
) -> (i32, i32) {
	let bytes = match ret {
		NetRet::OkBytes(b) => b,
		NetRet::OkStr(s) => s.into_bytes(),
		NetRet::WouldBlock => return (1, 0),
		NetRet::Err(e) => {
			ctx.state.last_error = e;
			return (2, 0);
		}
		NetRet::OkInt(_) | NetRet::OkNothing => unreachable!("net_bytes_v8 on a scalar op"),
	};
	let len = bytes.len().min(cap.max(0) as usize);
	write_mem(scope, mem, dst.max(0) as usize, &bytes[..len]);
	(0, len as i32)
}

fn net_dial(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	connect: bool,
	rv: &mut v8::ReturnValue,
) {
	let (ap, al) = (argi(scope, args, 0), argi(scope, args, 1));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let addr = read_str(scope, mem, ap, al);
	let ret = if connect {
		ctx.state.net.connect(&addr)
	} else {
		ctx.state.net.listen(&addr)
	};
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, rv, s, n);
}
fn cb_net_listen(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	net_dial(s, &a, false, &mut r);
}
fn cb_net_connect(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	net_dial(s, &a, true, &mut r);
}

fn cb_net_close(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let id = argi(scope, &args, 0) as u32;
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.close(id);
	rv.set_int32(net_scalar_v8(ctx, ret).0);
}

fn cb_net_local_addr(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let id = argi(scope, &args, 0) as u32;
	let (dst, cap) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.local_addr(id);
	let (s, n) = net_bytes_v8(scope, mem, ctx, dst, cap, ret);
	set_pair(scope, &mut rv, s, n);
}

fn cb_net_accept(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let lid = argi(scope, &args, 1) as u32;
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.try_accept(fid, lid);
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, &mut rv, s, n);
}

fn cb_net_read(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let cid = argi(scope, &args, 1) as u32;
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let ret = ctx.state.net.try_read(fid, cid, cap.max(0) as usize);
	let (s, n) = net_bytes_v8(scope, mem, ctx, dst, cap, ret);
	set_pair(scope, &mut rv, s, n);
}

fn cb_net_write(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let cid = argi(scope, &args, 1) as u32;
	let (src, len) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let data = read_mem(scope, mem, src.max(0) as usize, len.max(0) as usize);
	let ret = ctx.state.net.try_write(fid, cid, &data);
	let (s, n) = net_scalar_v8(ctx, ret);
	set_pair(scope, &mut rv, s, n);
}

/// `net-poll(i64 deadline) -> i32`: block until a parked socket is ready (the deadline
/// arrives as a JS BigInt).
fn cb_net_poll(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let deadline = args
		.get(0)
		.to_big_int(scope)
		.map(|b| b.i64_value().0)
		.unwrap_or(-1);
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	rv.set_int32(ctx.state.net.poll(deadline));
}

fn cb_net_unwatch(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let (ctx, _mem) = ctx_and_mem(scope, &args);
	ctx.state.net.unwatch(fid);
}

/// `float_to_str(f64, ptr, cap) -> i32 len`: format the float as `vm::Value`'s Display
/// does, write its UTF-8 bytes into scratch at `ptr` (≤ cap), return the length.
fn cb_float_to_str(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let n = args.get(0).number_value(scope).unwrap_or(0.0);
	let (ptr, cap) = (argi(scope, &args, 1), argi(scope, &args, 2));
	let s = if n.fract() == 0.0 && n.is_finite() {
		format!("{n:.1}")
	} else {
		format!("{n}")
	};
	let bytes = s.into_bytes();
	let (_ctx, mem) = ctx_and_mem(scope, &args);
	if bytes.len() <= cap.max(0) as usize {
		write_mem(scope, mem, ptr.max(0) as usize, &bytes);
	}
	rv.set_int32(bytes.len() as i32);
}
