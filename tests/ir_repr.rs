// Corpus harness for the IR Repr pass (see IR.md step 2 / `ir::repr`).
//
// The Repr pass (`infer_reprs` + `insert_coercions`) is a WASM-backend
// prerequisite that is *inert on the bytecode VM* — `Box`/`Unbox` lower to a
// no-op `Use`. That gives two VM anchors, both checked here over every fixture
// that lowers:
//
//   1. **Behavior neutrality.** Running the coercion-inserted program on the VM
//      must produce exactly the same status/stdout as running the un-coerced IR.
//      A coercion that corrupts a value (e.g. unboxing a record then feeding it
//      to `AddInt`) would fault at runtime and diverge — so this catches any
//      mis-placed coercion. (We compare against the *un-coerced IR* run, not the
//      AST reference, to isolate "did coercion change behavior" from "is the IR
//      path complete" — the latter is `ir_diff`'s job.)
//
//   2. **Repr discipline.** After coercion, `validate_reprs` must hold for every
//      function: no operand reaches a consumer that wants a different repr. This
//      is the static WASM-readiness check — the IR a WASM emitter will consume.
//
// Both run across *all* lowerable fixtures (including ones whose IR path still
// poisons unsupported constructs — the coercion transform and validator are
// well-defined on that IR too), so coverage doesn't depend on an allowlist.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use compiler::Compiler;

struct RunResult {
	status: String,
	stdout: String,
}

fn run_program(program: vm::Program) -> RunResult {
	let stdout = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program).with_stdout(vm::OutputSink::Buffer(stdout.clone()));
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

/// Count `Box`/`Unbox` rvalues in a function (recursing into nested blocks) —
/// used only to confirm the pass isn't a no-op.
fn count_coercions(f: &ir::Function) -> u32 {
	fn block(b: &ir::Block) -> u32 {
		let mut n = 0;
		for stmt in &b.0 {
			match &stmt.kind {
				ir::StmtKind::Let(_, ir::Rvalue::Box(_) | ir::Rvalue::Unbox(_, _)) => n += 1,
				ir::StmtKind::If(_, t, e) => n += block(t) + block(e),
				ir::StmtKind::Match { arms, .. } => n += arms.iter().map(|a| block(&a.body)).sum::<u32>(),
				ir::StmtKind::Switch { arms, default, .. } => {
					n += arms.iter().map(|(_, b)| block(b)).sum::<u32>() + block(default)
				}
				ir::StmtKind::Loop(b) => n += block(b),
				_ => {}
			}
		}
		n
	}
	block(&f.body)
}

#[test]
fn coercion_is_behavior_neutral_and_validates() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	// Anchor cwd so I/O fixtures write scratch files under the workspace target/.
	let _ = std::env::set_current_dir(workspace);
	let run_dir = workspace.join("tests/run");

	let mut dirs: Vec<_> = std::fs::read_dir(&run_dir)
		.unwrap()
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	dirs.sort();

	let mut checked = 0u32;
	let mut total_coercions = 0u32;
	for dir in &dirs {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let Some(compiler) = compile_check(dir) else {
			continue; // compile-error fixture; not relevant to the IR path
		};
		let Ok(uncoerced) = ir::lower(&compiler) else {
			continue; // lowering gap; ir_diff/coverage tracks these
		};

		// Reference behavior: the un-coerced IR path.
		let base = run_program(codegen::compile_from_ir(&uncoerced).expect("ir emit"));

		// Insert Repr coercions into every function, then validate the discipline.
		let mut coerced = uncoerced.clone();
		for f in &mut coerced.functions {
			ir::repr::insert_coercions(f);
			ir::repr::validate_reprs(f)
				.unwrap_or_else(|e| panic!("`{name}` fn `{}` fails repr validation: {e}", f.name));
			total_coercions += count_coercions(f);
		}

		// The coerced program must behave identically (Box/Unbox are VM no-ops).
		let after = run_program(codegen::compile_from_ir(&coerced).expect("coerced ir emit"));
		assert_eq!(
			(base.status.as_str(), base.stdout.as_str()),
			(after.status.as_str(), after.stdout.as_str()),
			"coercion changed behavior for `{name}`"
		);
		checked += 1;
	}

	// The pass must do real work — guard against it silently degrading to a no-op
	// (which would make the validator pass vacuously).
	assert!(
		total_coercions > 0,
		"expected the Repr pass to insert Box/Unbox coercions somewhere"
	);

	// Guard against the harness silently checking nothing.
	assert!(
		checked > 100,
		"expected many fixtures, only checked {checked}"
	);
}
