// Corpus harness for the IR monomorphization track (`ir::resolve`,
// `ir::mono`). Like `ir_repr`, these passes are WASM-backend
// prerequisites that are *inert on the bytecode VM*, so the VM anchors are
// behavior-neutrality (identical run output) plus a static check and a
// non-vacuity guard, run across every fixture that lowers.
//
// Phase 2 (here): **direct-call resolution.** Rewriting indirect calls to
// statically-known top-level functions into direct `Call(Callee::Function(..))`s
// must not change observable behavior — a top-level closure captures nothing, so
// the direct call is equivalent to loading the global and calling it. We compare
// against the *un-resolved IR* run (to isolate resolution from IR completeness,
// which is `ir_diff`'s job), and assert resolution actually fires somewhere.

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
	let mut vm_instance = vm::VM::new(program).with_stdout(vm::OutputSink::Buffer(stdout.clone())).with_stdin(vm::InputSource::Buffer(std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))));
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

/// Count direct `Call(Callee::Function(..))`s across a program (recursing into
/// nested blocks) — used to confirm resolution isn't a no-op.
fn count_direct_calls(program: &ir::IrProgram) -> u32 {
	fn block(b: &ir::Block) -> u32 {
		let mut n = 0;
		for stmt in &b.0 {
			match &stmt.kind {
				ir::StmtKind::Let(_, rv) | ir::StmtKind::Discard(rv) => {
					if matches!(rv, ir::Rvalue::Call(ir::Callee::Function(_), _)) {
						n += 1;
					}
				}
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
	program.functions.iter().map(|f| block(&f.body)).sum()
}

/// Count `Box`/`Unbox` rvalues in a function (recursing into nested blocks).
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

/// Did a function get an unboxed signature from monomorphization?
fn is_monomorphized(f: &ir::Function) -> bool {
	f.param_reprs.iter().any(|r| *r != ir::Repr::Boxed) || f.ret_repr != ir::Repr::Boxed
}

#[test]
fn direct_call_resolution_is_behavior_neutral() {
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
	let mut total_resolved = 0u32;
	for dir in &dirs {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let Some(compiler) = compile_check(dir) else {
			continue; // compile-error fixture; not relevant to the IR path
		};
		let Ok(base_ir) = ir::lower(&compiler) else {
			continue; // lowering gap; ir_diff/coverage tracks these
		};

		// Reference behavior: the un-resolved IR path.
		let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

		// Resolve direct calls, then run again — must match.
		let mut resolved = base_ir.clone();
		ir::resolve::resolve_direct_calls(&mut resolved);
		total_resolved += count_direct_calls(&resolved);
		let after = run_program(codegen::compile_from_ir(&resolved).expect("resolved ir emit"));

		assert_eq!(
			(base.status.as_str(), base.stdout.as_str()),
			(after.status.as_str(), after.stdout.as_str()),
			"direct-call resolution changed behavior for `{name}`"
		);
		checked += 1;
	}

	// Resolution must do real work — nearly every fixture calls `main` (and most
	// call other top-level defs) through a global, so this should be plentiful.
	assert!(
		total_resolved > 0,
		"expected direct-call resolution to rewrite some calls"
	);
	assert!(
		checked > 100,
		"expected many fixtures, only checked {checked}"
	);
}

#[test]
fn monomorphization_is_behavior_neutral_validates_and_reduces_coercions() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let run_dir = workspace.join("tests/run");

	let mut dirs: Vec<_> = std::fs::read_dir(&run_dir)
		.unwrap()
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	dirs.sort();

	let mut checked = 0u32;
	let mut monomorphized_fns = 0u32;
	// Programs where monomorphization strictly reduced total Box/Unbox churn. Not
	// every program improves: monomorphizing a function at a *boxed boundary* (a
	// helper called from boxed contexts, or a forced-`Boxed` `if`/`when` join)
	// relocates coercions to the call/join site, so corpus-wide totals can even
	// rise. The win concentrates in recursive/numeric chains where unboxed values
	// flow call-to-call (e.g. `fibonacci`). A profitability cost model that only
	// monomorphizes when it pays — and unifying a join's repr when all arms agree —
	// are explicit follow-ons; here we just prove the win is real *somewhere*.
	let mut improved = 0u32;
	let (mut corpus_uniform, mut corpus_mono) = (0u32, 0u32);
	for dir in &dirs {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let Some(compiler) = compile_check(dir) else {
			continue;
		};
		let Ok(base_ir) = ir::lower(&compiler) else {
			continue;
		};

		// Reference behavior: the plain IR path.
		let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

		// Baseline coercion count: uniform-boxed coercion of the un-monomorphized IR.
		let mut uni = base_ir.clone();
		let usigs = ir::repr::Sigs::uniform();
		let mut uniform_coercions = 0u32;
		for f in &mut uni.functions {
			f.var_reprs.clear();
			ir::repr::insert_coercions(f, &usigs);
			uniform_coercions += count_coercions(f);
		}

		// The full monomorphization track: resolve direct calls, decide eligibility
		// and filter signatures, then coerce + validate against the program's sigs.
		let mut mp = base_ir.clone();
		ir::mono::monomorphize(&mut mp);
		let msigs = ir::repr::Sigs::from_program(&mp);
		let mut mono_coercions = 0u32;
		for f in &mut mp.functions {
			f.var_reprs.clear();
			ir::repr::insert_coercions(f, &msigs);
			ir::repr::validate_reprs(f, &msigs)
				.unwrap_or_else(|e| panic!("`{name}` fn `{}` fails repr validation: {e}", f.name));
			mono_coercions += count_coercions(f);
			if is_monomorphized(f) {
				monomorphized_fns += 1;
			}
		}
		if mono_coercions < uniform_coercions {
			improved += 1;
		}
		corpus_uniform += uniform_coercions;
		corpus_mono += mono_coercions;

		// Monomorphizing only changes which (VM-inert) coercions are inserted, so
		// the run output must be unchanged.
		let after = run_program(codegen::compile_from_ir(&mp).expect("mono ir emit"));
		assert_eq!(
			(base.status.as_str(), base.stdout.as_str()),
			(after.status.as_str(), after.stdout.as_str()),
			"monomorphization changed behavior for `{name}`"
		);
		checked += 1;
	}

	// Non-vacuity + profitability: the pass monomorphizes functions, strictly
	// reduces coercions in at least one program, and — thanks to the self-recursive
	// + unboxed-param profitability proxy — NEVER increases the corpus-wide total
	// (monomorphizing only where an unboxed value rides the recursion).
	assert!(
		monomorphized_fns > 0,
		"expected some functions to be monomorphized"
	);
	assert!(
		improved > 0,
		"expected monomorphization to reduce coercions in at least one program"
	);
	assert!(
		corpus_mono <= corpus_uniform,
		"monomorphization must not increase corpus coercions: mono={corpus_mono} uniform={corpus_uniform}"
	);
	assert!(
		checked > 100,
		"expected many fixtures, only checked {checked}"
	);
}
