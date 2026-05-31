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

use compiler::{Compiler, Platform};
use wasmtime::{
	AnyRef, ArrayRef, ArrayRefPre, ArrayType, AsContextMut, Caller, Config, Engine, ExternType,
	FuncType, Instance, Linker, Module, RootScope, Rooted, Store, StructRef, StructRefPre,
	StructType, Val, ValType,
};

// Fixtures the WASM backend covers end-to-end today. Grow as coverage grows. The
// `core.io` filesystem/stdin fixtures are included under the **server platform** (see
// `compile_check` + compiler/src/platform.rs); their host glue is real `std::fs` + a
// stdin cursor.
//
// Intentionally NOT on this list (and why), so a future reader doesn't mistake them
// for unfinished work: `list-fold` / `list-pattern-if` are compile-error/warning
// fixtures (they never produce a VM run to diff against), and `builtin-unknown-tag`
// exercises the VM's "unknown builtin" *runtime* error — there's no builtin to
// import, so emit correctly rejects it. None are codegen gaps.
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
	"debug-passthrough",
	"defer-cleanup",
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
	// `core.io` filesystem + stdin (server platform — see compiler/src/platform.rs).
	// The host glue is real `std::fs` + a stdin cursor so the `err` strings and read
	// semantics match the VM; each runs idempotently (writes truncate, the rest clean
	// up) so the shared VM-then-wasm run against `target/…` paths stays consistent.
	"io-files",
	"io-read-missing",
	"io-append-delete",
	"io-make-dir",
	"io-read-dir",
	"io-bytes-roundtrip",
	"io-bytes-append",
	"io-bytes-non-utf8",
	"io-read-all",
	"io-read-eof",
	"io-read-lines",
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
	"list-sort",
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
	"time-basics",
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
	// async (Stage 1: single-fiber chain driver — sequential awaits, await-in-loop,
	// defer-on-failure, the sequential `then`/`or-else`/`map`/`attempt` combinators)
	"task-try-chain",
	"task-loop",
	"task-loop-bind",
	"task-defer",
	"task-fail",
	"task-combinators",
	"task-trait-poly",
	// async (Stage 2: scheduler — scopes/fibers/timers/cancellation)
	"scope-both",
	"scope-handle-param",
	"scope-deadline",
	"scope-race",
	"task-combinators-concurrent",
	"task-shielded",
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
	/// Program stdin (fed from the fixture's `stdin.txt`) + a read cursor — the
	/// `io-read`/`io-read-all` host imports drain it with the VM's line semantics.
	stdin: Rc<RefCell<Vec<u8>>>,
	stdin_pos: Rc<RefCell<usize>>,
	/// The message the last failed `core.io` call stashed (errno-style); returned
	/// by the `io-last-error` import, which `__io_result` queries on the err path.
	last_error: Rc<RefCell<String>>,
	/// The module's `$value` GC types, captured once from the witness the first io
	/// host import receives, so later calls build their returns without re-reflecting.
	gc_types: Rc<RefCell<Option<GcTypes>>>,
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
	let cell = caller.data().gc_types.clone();
	if let Some(t) = cell.borrow().clone() {
		return t;
	}
	let t = capture_gc_types(caller, witness);
	*cell.borrow_mut() = Some(t.clone());
	t
}

/// The cached GC types (set by the most recent io op's witness) — for `io-last-error`,
/// which carries no witness because it always follows a failing op that set them.
fn cached_types(caller: &Caller<HostState>) -> GcTypes {
	caller
		.data()
		.gc_types
		.borrow()
		.clone()
		.expect("io-last-error called before any io op cached the GC types")
}

/// Extract a `$str` argument as a Rust `String` (UTF-8 lossy, like the VM).
fn arg_string(store: &mut impl AsContextMut, v: &Val) -> String {
	String::from_utf8_lossy(&raw_value_bytes(store, v)).into_owned()
}

/// Read one line from the program's stdin buffer with the VM's `read_line`
/// semantics: `None` at EOF; otherwise the bytes up to the next `\n` (consumed),
/// with a trailing `\r` stripped.
fn stdin_line(state: &HostState) -> Option<String> {
	let buf = state.stdin.borrow();
	let mut pos = state.stdin_pos.borrow_mut();
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

/// Drain the rest of stdin as raw bytes (backs `read-all` / `read-all-bytes`).
fn stdin_rest(state: &HostState) -> Vec<u8> {
	let buf = state.stdin.borrow();
	let mut pos = state.stdin_pos.borrow_mut();
	let rest = buf[*pos..].to_vec();
	*pos = buf.len();
	rest
}

/// Stash an io error message for the next `io-last-error` query.
fn set_io_err(caller: &Caller<HostState>, msg: String) {
	*caller.data().last_error.borrow_mut() = msg;
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

struct RunResult {
	status: String,
	stdout: String,
}

/// Build an instance with the host imports wired (the `print` / `float_to_str` /
/// f64-math glue). The caller builds the `Engine` and compiles the `Module`
/// (cranelift JIT) once, so a benchmark loop can re-instantiate cheaply without
/// paying compilation each time. Returns the store (its `HostState.out`
/// accumulates printed bytes) and a fresh instance.
fn instantiate_module(
	engine: &Engine,
	module: &Module,
	stdin: &[u8],
) -> Result<(Store<HostState>, Instance), String> {
	let out = Rc::new(RefCell::new(Vec::<u8>::new()));
	let err = Rc::new(RefCell::new(Vec::<u8>::new()));
	let fail = Rc::new(RefCell::new(None));
	let mut store = Store::new(
		engine,
		HostState {
			out,
			err,
			fail,
			stdin: Rc::new(RefCell::new(stdin.to_vec())),
			stdin_pos: Rc::new(RefCell::new(0)),
			last_error: Rc::new(RefCell::new(String::new())),
			gc_types: Rc::new(RefCell::new(None)),
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
				},
			)
			.unwrap_or_else(|e| panic!("define {name}: {e}"));
	}
	// io.fail msg : stash the message, then trap. `run_wasm` reads the message
	// back to form the `runtime error: <msg>` status (mirrors the VM's abort).
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
				*caller.data().fail.borrow_mut() = Some(msg);
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
	// fs ops use real `std::fs` so the `err` strings match the VM's errno text. A real
	// server target (Rust/WASI) implements the same contract; a browser target omits
	// `core.io` entirely (gated by the platform — see compiler/src/platform.rs).
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
								set_io_err(&caller, e.to_string());
								Val::AnyRef(None)
							}
						}
					} else {
						match std::fs::read_to_string(&path) {
							Ok(s) => build_strlike(&mut caller, &gc, TAG_STR, s.as_bytes()),
							Err(e) => {
								set_io_err(&caller, e.to_string());
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
							set_io_err(&caller, e.to_string());
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
							set_io_err(&caller, e.to_string());
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
								set_io_err(&caller, msg);
								Val::AnyRef(None)
							}
							None => {
								names.sort();
								build_str_list(&mut caller, &gc, &names)
							}
						}
					}
					Err(e) => {
						set_io_err(&caller, e.to_string());
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
				results[0] = match stdin_line(caller.data()) {
					Some(line) => build_strlike(&mut caller, &gc, TAG_STR, line.as_bytes()),
					None => {
						set_io_err(&caller, "EOF".to_string());
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
				let bytes = stdin_rest(caller.data());
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
				let bytes = stdin_rest(caller.data());
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
				let msg = caller.data().last_error.borrow().clone();
				results[0] = build_strlike(&mut caller, &gc, TAG_STR, msg.as_bytes());
				Ok(())
			},
		)
		.expect("define io-last-error");

	let instance = linker
		.instantiate(&mut store, module)
		.map_err(|e| format!("instantiate error: {e}"))?;
	Ok((store, instance))
}

/// Build a fresh instance and run `_entry` once, collecting stdout (the diff path).
fn run_wasm(bytes: &[u8], stdin: &[u8]) -> RunResult {
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

/// Instantiate a pre-compiled module and run `_entry` once, collecting stdout.
/// Split out of `run_wasm` so a benchmark can re-instantiate a module that was
/// cranelift-compiled once, keeping JIT compilation out of the timed loop.
fn run_entry(engine: &Engine, module: &Module, stdin: &[u8]) -> RunResult {
	let (mut store, instance) = match instantiate_module(engine, module, stdin) {
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

fn run_vm(program: vm::Program, stdin: &[u8]) -> RunResult {
	let stdout = Rc::new(RefCell::new(Vec::<u8>::new()));
	let stderr = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program)
		.with_stdin(vm::InputSource::Buffer(Rc::new(RefCell::new(
			stdin.to_vec(),
		))))
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
	// The WASM backend targets the server platform here: it provides every
	// capability except the browser-only ones (Dom/Fetch/Timer), so `core.io`
	// resolves and no fixture in this corpus is gated.
	let mut compiler = Compiler::from_entry_path(dir.to_str().unwrap().to_string())
		.ok()?
		.with_platform(Platform::Server);
	vm::stdlib::register_compiler(&mut compiler);
	compiler.check().ok()?;
	Some(compiler)
}

#[test]
fn wasm_path_matches_reference() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	for name in WASM_FIXTURES {
		let dir = workspace.join("tests/run").join(name);
		// Feed the fixture's `stdin.txt` (empty if absent) to BOTH engines, so the
		// `io.read`/`io.read-all` fixtures see identical input.
		let stdin = std::fs::read(dir.join("stdin.txt")).unwrap_or_default();
		let compiler = compile_check(&dir).unwrap_or_else(|| panic!("compile failed for `{name}`"));
		let ir_program = ir::lower(&compiler).unwrap_or_else(|e| panic!("ir::lower `{name}`: {e}"));
		let reference = run_vm(
			codegen::compile_from_ir(&ir_program).expect("reference compile"),
			&stdin,
		);
		let bytes = wasm::emit(&ir_program).unwrap_or_else(|d| panic!("wasm::emit `{name}`: {:?}", d));
		let via_wasm = run_wasm(&bytes, &stdin);
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
		let stdin = std::fs::read(dir.join("stdin.txt")).unwrap_or_default();
		let Some(compiler) = compile_check(dir) else {
			continue;
		};
		let Ok(ir_program) = ir::lower(&compiler) else {
			continue;
		};
		let reference = run_vm(
			codegen::compile_from_ir(&ir_program).expect("reference compile"),
			&stdin,
		);
		// Emit can panic on a not-yet-handled construct; catch so the scan finishes.
		let emitted =
			std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wasm::emit(&ir_program)));
		match emitted {
			Ok(Ok(bytes)) => {
				let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_wasm(&bytes, &stdin)));
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

// ---------------------------------------------------------------------------
// VM-vs-WASM execution benchmark.
//
// For each program under `benchmarks/programs/<name>/main.pa`, compile it once
// per backend (VM bytecode / cranelift-compiled WasmGC module), confirm the two
// backends agree, then time only *execution*: the VM run (a fresh `vm::Program`
// clone per iteration, so memoized globals reset) versus re-instantiating the
// pre-compiled wasm module and calling `_entry`. JIT/codegen is hoisted out of
// the timed loops, so this measures the interpreters/compiled code, not the
// compilers. Run with:
//
//   cargo test -p tests --test wasm_diff bench_vm_vs_wasm -- --ignored --nocapture
//   BENCH_ITERS=50 cargo test -p tests --test wasm_diff bench_vm_vs_wasm -- --ignored --nocapture
// ---------------------------------------------------------------------------

#[test]
#[ignore = "VM-vs-WASM benchmark; run with --ignored --nocapture"]
fn bench_vm_vs_wasm() {
	// A roomy stack: the VM nests a Rust frame per Pluma call (no TCO on the
	// bytecode path), so the deep-recursion benchmarks would overflow the test
	// harness's default 2 MiB thread otherwise.
	std::thread::Builder::new()
		.stack_size(256 * 1024 * 1024)
		.spawn(bench_body)
		.unwrap()
		.join()
		.unwrap();
}

fn fmt_dur(d: std::time::Duration) -> String {
	let us = d.as_secs_f64() * 1_000_000.0;
	if us < 1000.0 {
		format!("{us:.1} us")
	} else if us < 1_000_000.0 {
		format!("{:.2} ms", us / 1000.0)
	} else {
		format!("{:.3} s", us / 1_000_000.0)
	}
}

fn bench_body() {
	use std::time::{Duration, Instant};

	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let programs_dir = workspace.join("benchmarks/programs");

	let iters: u32 = std::env::var("BENCH_ITERS")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(20);
	// The null collector (allocate, never free) is fastest and fits as long as a
	// single run's live set stays under wasmtime's GC heap; `BENCH_WASM_GC=drc`
	// switches to the deferred-reference-counting collector, which reclaims within
	// a run, for allocation-heavy programs that would otherwise trap on OOM.
	let use_drc = std::env::var("BENCH_WASM_GC").as_deref() == Ok("drc");

	let mut entries: Vec<_> = std::fs::read_dir(&programs_dir)
		.expect("benchmarks/programs not found")
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	entries.sort();

	println!("\nVM vs WASM execution time  ({iters} iterations, average per run)\n");
	println!(
		"{:<16}  {:>12}  {:>12}  {:>9}",
		"benchmark", "VM", "WASM", "WASM/VM"
	);
	println!("{:-<16}  {:->12}  {:->12}  {:->9}", "", "", "", "");

	for dir in &entries {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let stdin = std::fs::read(dir.join("stdin.txt")).unwrap_or_default();

		let note = |msg: &str| println!("{name:<16}  {msg}");

		let Some(compiler) = compile_check(dir) else {
			note("compile error — skipped");
			continue;
		};
		let ir_program = match ir::lower(&compiler) {
			Ok(p) => p,
			Err(e) => {
				note(&format!("ir::lower error: {e} — skipped"));
				continue;
			}
		};

		// VM artifact: compiled once, cloned per run (a run memoizes globals).
		let vm_program = match codegen::compile_from_ir(&ir_program) {
			Ok(p) => p,
			Err(e) => {
				note(&format!("VM compile error: {e} — skipped"));
				continue;
			}
		};

		// WASM artifact: emit + cranelift-compile the module once, up front.
		let bytes = match wasm::emit(&ir_program) {
			Ok(b) => b,
			Err(d) => {
				note(&format!("wasm::emit error ({} diag) — skipped", d.0.len()));
				continue;
			}
		};
		let wasm_engine = if use_drc { bench_engine() } else { engine() };
		let module = match Module::new(&wasm_engine, &bytes) {
			Ok(m) => m,
			Err(e) => {
				note(&format!("wasm module error: {e} — skipped"));
				continue;
			}
		};

		// Sanity: don't time two programs that compute different things.
		let vm_ref = run_vm(vm_program.clone(), &stdin);
		let wasm_ref = run_entry(&wasm_engine, &module, &stdin);
		if vm_ref.status != wasm_ref.status || vm_ref.stdout != wasm_ref.stdout {
			note(&format!(
				"OUTPUT MISMATCH — vm=({:?}, {:?})  wasm=({:?}, {:?})",
				vm_ref.status,
				vm_ref.stdout.trim_end(),
				wasm_ref.status,
				wasm_ref.stdout.trim_end()
			));
			continue;
		}

		// Time the VM. The per-iteration `clone` is outside the timer.
		let _ = run_vm(vm_program.clone(), &stdin); // warm up
		let mut vm_total = Duration::ZERO;
		for _ in 0..iters {
			let p = vm_program.clone();
			let start = Instant::now();
			let _ = run_vm(p, &stdin);
			vm_total += start.elapsed();
		}
		let vm_avg = vm_total / iters;

		// Time the WASM: re-instantiate the pre-compiled module + call `_entry`.
		let _ = run_entry(&wasm_engine, &module, &stdin); // warm up
		let mut wasm_total = Duration::ZERO;
		for _ in 0..iters {
			let start = Instant::now();
			let _ = run_entry(&wasm_engine, &module, &stdin);
			wasm_total += start.elapsed();
		}
		let wasm_avg = wasm_total / iters;

		let ratio = wasm_avg.as_secs_f64() / vm_avg.as_secs_f64();
		println!(
			"{:<16}  {:>12}  {:>12}  {:>8.2}x",
			name,
			fmt_dur(vm_avg),
			fmt_dur(wasm_avg),
			ratio
		);
	}
	println!("\n(WASM/VM < 1.00x means the WasmGC backend ran faster.)");
}
