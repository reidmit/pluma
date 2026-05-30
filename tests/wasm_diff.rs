// Differential harness for the WASM (WasmGC) backend. For each
// allowlisted fixture, compile it the reference way (`ir::lower` →
// `codegen::compile_from_ir` → VM) and the WASM way (`ir::lower` → `wasm::emit`
// → run in wasmtime with Rust host glue), and assert identical stdout + status.
//
// The host glue (the `print` import + value formatting) is written in Rust here,
// mirroring `vm::Value`'s `Display`; it is the throwaway test-only host (the
// browser target reimplements the same contract in JS). The `tag` constants are
// the cross-cutting contract with `wasm::types`.
//
// `WASM_FIXTURES` grows as coverage grows; `wasm_coverage_report` (ignored) scans
// every fixture and reports which the WASM path already reproduces.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use compiler::Compiler;
use wasmtime::{
	AnyRef, AsContextMut, Config, Engine, FuncType, Instance, Linker, Module, RootScope, Rooted,
	Store, Val, ValType,
};

// Fixtures the WASM backend covers end-to-end today. Grow as coverage grows.
const WASM_FIXTURES: &[&str] = &[
	"regex-matches",
	"regex-find",
	"regex-anchors",
	"regex-character-classes",
	"regex-quantifiers",
	"regex-quantifier-shapes",
	"regex-alternation",
	"regex-named-captures",
	"regex-named-capture-lookup",
	"regex-replace",
	"regex-split",
	"regex-as-alias",
	"arith-precedence",
	"arithmetic",
	"bare-trait-methods",
	"base64-roundtrip",
	"builtin-uses-list-length",
	"bytes-equality",
	"bytes-escapes",
	"bytes-hash-ord",
	"bytes-literal",
	"bytes-module-basics",
	"bytes-module-from-list",
	"bytes-module-search",
	"bytes-module-split-join",
	"bytes-pattern",
	"bytes-string-bridge",
	"closures",
	"closures-in-list",
	"coalesce-chain",
	"coalesce-option",
	"coalesce-result",
	"comparison-ops",
	"core-dict-basic",
	"core-dict-collision",
	"core-dict-derived",
	"core-dict-fold",
	"core-dict-from-entries",
	"core-dict-int-keys",
	"core-dict-merge",
	"core-dict-string-keys",
	"core-list-extras",
	"core-math-extras",
	"core-string",
	"cross-module",
	"dict-equality",
	"dict-tostring",
	"deep-recursion",
	"double-int-float",
	"duration-literals",
	"else-if-chain",
	"empty-fun-body",
	"equality-structural",
	"expect-err",
	"expect-none",
	"expect-passthrough",
	"factorial",
	"fibonacci",
	"float-arith",
	"float-compare",
	"float-nan-compare",
	"generic-enum",
	"hash-trait",
	"hello",
	"hex-roundtrip",
	"if-else-pattern",
	"if-else-value",
	"if-no-match",
	"fail-direct",
	"io-print",
	"io-write-bytes",
	"interpolation-complex",
	"interpolation-nested-string",
	"json-basic",
	"json-error",
	"json-pretty",
	"json-walkers",
	"let-destructure-record",
	"let-destructure-tuple",
	"let-destructure-underscore",
	"let-in-when",
	"let-then-pattern",
	"let-type-annotation",
	"list-chained",
	"list-contains",
	"list-each",
	"list-length",
	"list-map-filter",
	"list-pattern-anonymous-rest",
	"list-pattern-basic",
	"list-pattern-exact",
	"list-pattern-nested",
	"list-pattern-recursive-sum",
	"list-pattern-rest-type",
	"list-core-combinators",
	"list-extended-combinators",
	"list-reverse-concat",
	"list-set",
	"list-sort-explicit-cmp",
	"list-spread",
	"list-spread-record-update",
	"main-returns-err",
	"main-returns-ok",
	"main-try-propagates",
	"math-builtins",
	"mutual-recursion",
	"negative-numbers",
	"nested-enum",
	"option-then-direct",
	"ord-compare-wrappers",
	"ord-operators",
	"partial-application",
	"partial-record-match",
	"pattern-stack-cleanup",
	"pipeline",
	"prelude-option",
	"prelude-parametric",
	"quadruple-forwarding",
	"record-field-shorthand",
	"record-list-cross-nesting",
	"record-pattern",
	"record-pattern-closed-vs-open",
	"record-pattern-named-rest",
	"record-pattern-nested-rest",
	"record-pattern-row-poly",
	"record-update",
	"recursion",
	"ref-basic",
	"ref-tostring",
	"result-then-direct",
	"shadowing",
	"string-concat",
	"string-literal-pattern",
	"string-parse",
	"string-module-split-join",
	"string-slice",
	"string-with-escapes",
	"subtract-after-call",
	"swap-tuple",
	"top-level-keywords",
	"to-string-shapes",
	"trait-dict-forward-recursive",
	"trait-fn-as-value",
	"tuple-element-access",
	"tuple-pattern-size",
	"try-nested",
	"try-option",
	"try-result",
	"try-wildcard",
	"unary-minus",
	"user-trait-concrete",
	"user-trait-default",
	"user-trait-parametric",
	"variant-as-value",
	"variant-with-record-arg",
	"visibility",
	"wasm-math-trig",
	"when-else",
	"when-enum",
	"wire-dict",
	"wire-fingerprint",
	"wire-polymorphic",
	"wire-recursive",
	"wire-roundtrip",
];

// Runtime tags — must match `wasm::types`.
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

struct HostState {
	out: Rc<RefCell<Vec<u8>>>,
	/// stderr sink — collected separately so it never pollutes the stdout the
	/// differential compares (the reference `run_vm` drops stderr too).
	err: Rc<RefCell<Vec<u8>>>,
	/// The `io.fail` abort message, stashed before the host traps so `run_wasm`
	/// can surface it as the program's `runtime error: <msg>` status.
	fail: Rc<RefCell<Option<String>>>,
}

fn engine() -> Engine {
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
			let f = s.field(&mut *store, 1).expect("list elems");
			format!("[{}]", format_elems(store, &f).join(", "))
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
	let s = r.as_struct(&mut *store).expect("as_struct").expect("a $value");
	let arr = match s.field(&mut *store, 1).expect("bytes field") {
		Val::AnyRef(Some(a)) => a.as_array(&mut *store).expect("as_array").expect("bytes array"),
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

struct RunResult {
	status: String,
	stdout: String,
}

/// Build an instance with the host imports wired (the `print` / `float_to_str` /
/// f64-math glue). `use_drc` picks the collecting drc GC (for the allocation-heavy
/// timed bench loop) over the default never-collecting null GC. Returns the store
/// (its `HostState.out` accumulates printed bytes) and the instance.
fn instantiate_module(use_drc: bool, bytes: &[u8]) -> Result<(Store<HostState>, Instance), String> {
	let engine = if use_drc { bench_engine() } else { engine() };
	let module = Module::new(&engine, bytes).map_err(|e| format!("module error: {e}"))?;
	let out = Rc::new(RefCell::new(Vec::<u8>::new()));
	let err = Rc::new(RefCell::new(Vec::<u8>::new()));
	let fail = Rc::new(RefCell::new(None));
	let mut store = Store::new(&engine, HostState { out, err, fail });
	let mut linker: Linker<HostState> = Linker::new(&engine);
	// print : (ref null $value) -> ()  — host accepts the broader `anyref`.
	let print_ty = FuncType::new(&engine, [ValType::ANYREF], []);
	linker
		.func_new("pluma", "print", print_ty, |mut caller, args, _results| {
			// Bound the temporary GC roots created while reflecting the value: a
			// `RootScope` frees them on drop, so repeated prints don't accumulate
			// roots (which otherwise corrupts wasmtime's GC under collection).
			let line = {
				let mut scope = RootScope::new(&mut caller);
				format_value(&mut scope, &args[0])
			};
			let buf = caller.data().out.clone();
			buf.borrow_mut().extend_from_slice(line.as_bytes());
			buf.borrow_mut().push(b'\n');
			Ok(())
		})
		.expect("define print");
	// The `core.io` writers. `print`/`print-err` append a newline; `write`/
	// `write-err` don't. The `*-err` pair targets the stderr sink. `*-bytes`
	// write a `bytes` value's raw bytes (no Display formatting).
	let io_ty = FuncType::new(&engine, [ValType::ANYREF], []);
	for (name, to_err, newline, raw) in [
		("io-print", false, true, false),
		("io-write", false, false, false),
		("io-print-err", true, true, false),
		("io-write-err", true, false, false),
		("io-write-bytes", false, false, true),
		("io-write-err-bytes", true, false, true),
	] {
		linker
			.func_new("pluma", name, io_ty.clone(), move |mut caller, args, _results| {
				let bytes = {
					let mut scope = RootScope::new(&mut caller);
					if raw {
						raw_value_bytes(&mut scope, &args[0])
					} else {
						format_value(&mut scope, &args[0]).into_bytes()
					}
				};
				let sink = if to_err {
					caller.data().err.clone()
				} else {
					caller.data().out.clone()
				};
				sink.borrow_mut().extend_from_slice(&bytes);
				if newline {
					sink.borrow_mut().push(b'\n');
				}
				Ok(())
			})
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}
	// io.fail msg : stash the message, then trap. `run_wasm` reads the message
	// back to form the `runtime error: <msg>` status (mirrors the VM's abort).
	linker
		.func_new("pluma", "io-fail", io_ty.clone(), |mut caller, args, _results| {
			let msg = {
				let mut scope = RootScope::new(&mut caller);
				format_value(&mut scope, &args[0])
			};
			*caller.data().fail.borrow_mut() = Some(msg);
			Err(wasmtime::Error::msg("io.fail"))
		})
		.expect("define io-fail");
	// float_to_str : (f64, $bytes buf) -> i32 len. Format the float as `vm::Value`'s
	// Display does, write the bytes into the caller-provided GC byte array, return
	// the length. (A real browser target would delegate to JS similarly.)
	let f2s_ty = FuncType::new(&engine, [ValType::F64, ValType::ANYREF], [ValType::I32]);
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
	let f64_unary_ty = FuncType::new(&engine, [ValType::F64], [ValType::F64]);
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

	let instance = linker
		.instantiate(&mut store, &module)
		.map_err(|e| format!("instantiate error: {e}"))?;
	Ok((store, instance))
}

/// Build a fresh instance and run `_entry` once, collecting stdout (the diff path).
fn run_wasm(bytes: &[u8]) -> RunResult {
	let (mut store, instance) = match instantiate_module(false, bytes) {
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
		// aborts with `e` on stderr and a nonzero exit (mirrors `vm::VM::run`).
		Ok(_) => match err_message(&mut store, &results[0]) {
			Some(msg) => format!("runtime error: {msg}"),
			None => "ok".to_string(),
		},
		// A trap with a stashed `io.fail` message is a program-controlled abort;
		// surface its message (matching the VM) rather than the wasm backtrace.
		Err(e) => match store.data().fail.borrow().clone() {
			Some(msg) => format!("runtime error: {msg}"),
			None => format!("runtime error: {e}"),
		},
	};
	let stdout = String::from_utf8_lossy(&store.data().out.borrow()).into_owned();
	RunResult { status, stdout }
}

/// A collecting (deferred-reference-counting) engine for the bench: the timed loop
/// allocates a record per iteration, which the default null collector would never
/// free (OOM). The short-lived records are reclaimed within each `_entry` call.
fn bench_engine() -> Engine {
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

fn run_vm(program: vm::Program) -> RunResult {
	let stdout = Rc::new(RefCell::new(Vec::<u8>::new()));
	let stderr = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program).with_stdin(vm::InputSource::Buffer(std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))))
		.with_stdout(vm::OutputSink::Buffer(stdout.clone()))
		.with_stderr(vm::OutputSink::Buffer(stderr.clone()));
	let status = match vm_instance.run() {
		Ok(_) => "ok".to_string(),
		Err(e) => format!("runtime error: {}", e.message),
	};
	let out = String::from_utf8_lossy(&stdout.borrow()).into_owned();
	RunResult {
		status,
		stdout: out,
	}
}

fn compile_check(dir: &Path) -> Option<Compiler> {
	let mut compiler = Compiler::from_entry_path(dir.to_str().unwrap().to_string()).ok()?;
	vm::stdlib::register_compiler(&mut compiler);
	compiler.check().ok()?;
	Some(compiler)
}

#[test]
fn wasm_path_matches_reference() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	for name in WASM_FIXTURES {
		let dir = workspace.join("tests/run").join(name);
		let compiler = compile_check(&dir).unwrap_or_else(|| panic!("compile failed for `{name}`"));
		let ir_program = ir::lower(&compiler).unwrap_or_else(|e| panic!("ir::lower `{name}`: {e}"));
		let reference = run_vm(codegen::compile_from_ir(&ir_program).expect("reference compile"));
		let bytes = wasm::emit(&ir_program).unwrap_or_else(|d| panic!("wasm::emit `{name}`: {:?}", d));
		let via_wasm = run_wasm(&bytes);
		assert_eq!(
			reference.status, via_wasm.status,
			"status mismatch for `{name}`"
		);
		assert_eq!(
			reference.stdout, via_wasm.stdout,
			"stdout mismatch for `{name}`"
		);
	}
}

#[test]
#[ignore = "coverage report; run with --ignored --nocapture"]
fn wasm_coverage_report() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let run_dir = workspace.join("tests/run");
	let mut entries: Vec<_> = std::fs::read_dir(&run_dir)
		.unwrap()
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	entries.sort();

	let (mut matching, mut diff, mut emit_err, mut panicked) = (Vec::new(), 0u32, 0u32, Vec::new());
	for dir in &entries {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let Some(compiler) = compile_check(dir) else {
			continue;
		};
		let Ok(ir_program) = ir::lower(&compiler) else {
			continue;
		};
		let reference = run_vm(codegen::compile_from_ir(&ir_program).expect("reference compile"));
		// Emit can panic on a not-yet-handled construct; catch so the scan finishes.
		let emitted =
			std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wasm::emit(&ir_program)));
		match emitted {
			Ok(Ok(bytes)) => {
				let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_wasm(&bytes)));
				match r {
					Ok(w) if w.status == reference.status && w.stdout == reference.stdout => {
						matching.push(name)
					}
					Ok(w) => {
						diff += 1;
						if std::env::var("WASM_DUMP_DIFF").is_ok() {
							eprintln!(
								"DIFF {name}:\n  ref status={:?} stdout={:?}\n  wasm status={:?} stdout={:?}",
								reference.status, reference.stdout, w.status, w.stdout
							);
						}
					}
					Err(_) => panicked.push(name),
				}
			}
			Ok(Err(_)) => emit_err += 1,
			Err(_) => panicked.push(name),
		}
	}
	let total = matching.len() as u32 + diff + emit_err + panicked.len() as u32;
	println!(
		"\nWASM coverage: {} match / {} diff / {} emit-err / {} PANIC  (of {} runnable fixtures)",
		matching.len(),
		diff,
		emit_err,
		panicked.len(),
		total
	);
	if !panicked.is_empty() {
		println!("PANICKING fixtures:");
		for n in &panicked {
			println!("  {n}");
		}
	}
	println!("matching fixtures:");
	for n in &matching {
		println!("  {n}");
	}
}
