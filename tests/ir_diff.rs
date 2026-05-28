// Differential harness for the IR codegen path (see IR.md, phase 1.2/1.3).
//
// For each allowlisted fixture, compile it BOTH ways — the existing
// AST->bytecode path (`codegen::compile`) and the new IR path
// (`ir::lower` + `codegen::compile_from_ir`) — run both on the VM capturing
// output, and assert identical observable behavior. The allowlist grows as
// `ir::lower`'s construct coverage grows; a fixture whose executed path still
// hits a poison thunk simply isn't listed yet.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use compiler::Compiler;

// Fixtures (under tests/run/<name>/) that `ir::lower` covers end-to-end today.
// Grow this as coverage grows; `ir_coverage_report` (below) lists candidates.
const IR_FIXTURES: &[&str] = &[
	"arith-precedence",
	"arithmetic",
	"bare-trait-methods",
	"builtin-unknown-tag",
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
	"core-dict-fold",
	"core-dict-from-entries",
	"core-dict-int-keys",
	"core-dict-merge",
	"core-dict-string-keys",
	"core-list-extras",
	"core-math-extras",
	"core-string",
	"cross-module",
	"debug-passthrough",
	"deep-recursion",
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
	"fail-direct",
	"fibonacci",
	"float-arith",
	"float-compare",
	"float-nan-compare",
	"generic-enum",
	"hash-trait",
	"hello",
	"if-else-pattern",
	"if-else-value",
	"if-no-match",
	"interpolation-complex",
	"interpolation-nested-string",
	"io-append-delete",
	"io-bytes-append",
	"io-bytes-non-utf8",
	"io-bytes-roundtrip",
	"io-files",
	"io-make-dir",
	"io-print",
	"io-read-all",
	"io-read-dir",
	"io-read-eof",
	"io-read-lines",
	"io-read-missing",
	"io-write-bytes",
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
	"list-reverse-concat",
	"list-sort",
	"list-spread",
	"main-returns-err",
	"main-returns-ok",
	"main-try-propagates",
	"math-builtins",
	"mutual-recursion",
	"negative-numbers",
	"nested-enum",
	"option-then-direct",
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
	"recursion",
	"ref-basic",
	"regex-alternation",
	"regex-anchors",
	"regex-as-alias",
	"regex-character-classes",
	"regex-find",
	"regex-matches",
	"regex-named-capture-lookup",
	"regex-named-captures",
	"regex-quantifier-shapes",
	"regex-quantifiers",
	"regex-replace",
	"regex-split",
	"result-then-direct",
	"scope-both",
	"scope-deadline",
	"scope-handle-param",
	"scope-race",
	"shadowing",
	"string-concat",
	"string-literal-pattern",
	"string-parse",
	"string-slice",
	"string-with-escapes",
	"subtract-after-call",
	"swap-tuple",
	"task-combinators",
	"task-combinators-concurrent",
	"task-defer",
	"task-fail",
	"task-loop",
	"task-loop-bind",
	"task-shielded",
	"task-trait-poly",
	"task-try-chain",
	"time-basics",
	"to-string-shapes",
	"top-level-keywords",
	"trait-fn-as-value",
	"try-nested",
	"try-option",
	"try-result",
	"try-wildcard",
	"tuple-pattern-size",
	"unary-minus",
	"user-trait-concrete",
	"user-trait-default",
	"user-trait-parametric",
	"variant-as-value",
	"variant-with-record-arg",
	"visibility",
	"when-else",
	"when-enum",
];

struct RunResult {
	status: String,
	stdout: String,
}

fn run_program(program: vm::Program) -> RunResult {
	let stdout = Rc::new(RefCell::new(Vec::<u8>::new()));
	let stderr = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program)
		.with_stdout(vm::OutputSink::Buffer(stdout.clone()))
		.with_stderr(vm::OutputSink::Buffer(stderr.clone()));
	let status = match vm_instance.run() {
		Ok(_) => "ok".to_string(),
		Err(e) => format!("runtime error: {}", e.message),
	};
	let out = String::from_utf8_lossy(&stdout.borrow()).to_string();
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

// Scans every tests/run fixture, compiles it both ways, and reports which ones
// the IR path already reproduces. Not part of the default run — it's discovery
// tooling for growing IR_FIXTURES. Run with:
//   cargo test -p tests --test ir_diff -- --ignored --nocapture ir_coverage
#[test]
#[ignore = "coverage report; run with --ignored --nocapture"]
fn ir_coverage_report() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	// Anchor cwd at the workspace root so I/O fixtures write their scratch
	// files under the (gitignored) workspace `target/`, not next to the crate.
	let _ = std::env::set_current_dir(workspace);
	let run_dir = workspace.join("tests/run");
	let mut matching = Vec::new();
	let (mut diff, mut lower_err) = (0u32, 0u32);

	let mut entries: Vec<_> = std::fs::read_dir(&run_dir)
		.unwrap()
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	entries.sort();

	let mut panicked = Vec::new();
	for dir in &entries {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let Some(compiler) = compile_check(dir) else {
			continue; // compile-error fixture; not relevant to the IR path
		};
		let reference = run_program(codegen::compile(&compiler).expect("reference compile"));
		match ir::lower(&compiler) {
			Ok(ir_program) => {
				// Mis-lowered bytecode can trip a VM `unreachable!`; catch it so the
				// report finishes and names the offender instead of crashing.
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					run_program(codegen::compile_from_ir(&ir_program).expect("ir emit"))
				}));
				match result {
					Ok(via_ir) if via_ir.status == reference.status && via_ir.stdout == reference.stdout => {
						matching.push(name)
					}
					Ok(via_ir) => {
						diff += 1;
						if std::env::var("IR_DUMP_DIFF").is_ok() {
							eprintln!(
								"DIFF {name}:\n  ref status={:?} stdout={:?}\n  ir  status={:?} stdout={:?}",
								reference.status, reference.stdout, via_ir.status, via_ir.stdout
							);
						}
					}
					Err(_) => panicked.push(name),
				}
			}
			Err(_) => lower_err += 1,
		}
	}

	let total = matching.len() as u32 + diff + lower_err + panicked.len() as u32;
	println!(
		"\nIR coverage: {} match / {} diff / {} lower-err / {} PANIC  (of {} runnable fixtures)",
		matching.len(),
		diff,
		lower_err,
		panicked.len(),
		total
	);
	if !panicked.is_empty() {
		println!("PANICKING fixtures (mis-lowering bugs):");
		for name in &panicked {
			println!("  {name}");
		}
	}
	println!("matching fixtures:");
	for name in &matching {
		println!("  {name}");
	}
}

#[test]
fn ir_path_matches_reference() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	for name in IR_FIXTURES {
		let dir = workspace.join("tests/run").join(name);

		let mut compiler = Compiler::from_entry_path(dir.to_str().unwrap().to_string())
			.unwrap_or_else(|_| panic!("from_entry_path failed for `{name}`"));
		vm::stdlib::register_compiler(&mut compiler);
		compiler
			.check()
			.unwrap_or_else(|_| panic!("check failed for `{name}`"));

		// Reference: the existing AST->bytecode path.
		let reference = run_program(codegen::compile(&compiler).expect("reference compile"));

		// Under test: AST -> IR -> bytecode.
		let ir_program =
			ir::lower(&compiler).unwrap_or_else(|e| panic!("ir::lower failed for `{name}`: {e}"));
		let via_ir = run_program(codegen::compile_from_ir(&ir_program).expect("ir emit"));

		assert_eq!(
			reference.status, via_ir.status,
			"status mismatch for `{name}`"
		);
		assert_eq!(
			reference.stdout, via_ir.stdout,
			"stdout mismatch for `{name}`"
		);
	}
}
