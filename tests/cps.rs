// Differential harness for the async CPS state-machine pass (`ir::cps`) + the
// VM poll-driver (`vm::task::drive_poll`). Like the other IR-track passes the
// transform is *inert on the default
// VM path* and WASM-bound, so the VM anchors are behavior-neutrality plus a
// non-vacuity guard.
//
// The pass rewrites supported `is_async` functions into poll form and points the
// VM at a second stepper (`drive_poll`) that advances them by *calling* a
// generated `poll(state, resume)` instead of snapshotting the frame. Both
// steppers share the one scheduler, so a corpus run with the pass applied must
// produce byte-identical output to the plain (Await-style) IR run — and the pass
// must actually transform something.

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

/// Every fixture under `tests/run/` that compiles, sorted. cwd is anchored to the
/// workspace root so I/O fixtures write scratch under `target/`.
fn run_fixtures() -> Vec<std::path::PathBuf> {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let run_dir = workspace.join("tests/run");
	let mut dirs: Vec<_> = std::fs::read_dir(&run_dir)
		.unwrap()
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	dirs.sort();
	dirs
}

fn poll_transformed_count(program: &ir::IrProgram) -> u32 {
	program
		.functions
		.iter()
		.filter(|f| f.poll_fn.is_some())
		.count() as u32
}

// Applying the CPS pass + running through the poll-driver is byte-for-byte
// equivalent to the plain (Await-style) IR run, across the whole corpus — and the
// pass transforms at least one function somewhere (non-vacuity).
#[test]
fn cps_transform_is_behavior_neutral() {
	let dirs = run_fixtures();
	let mut checked = 0u32;
	let mut total_transformed = 0u32;
	for dir in &dirs {
		let name = dir.file_name().unwrap().to_string_lossy().to_string();
		let Some(compiler) = compile_check(dir) else {
			continue; // compile-error fixture; not relevant to the IR path
		};
		let Ok(base_ir) = ir::lower(&compiler) else {
			continue; // lowering gap; ir_diff/coverage tracks these
		};

		// Reference behavior: the plain IR path (every async fn Await-style).
		let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

		// Transform to poll style, then run again — must match byte-for-byte.
		let mut cps = base_ir.clone();
		ir::cps::cps_transform(&mut cps);
		total_transformed += poll_transformed_count(&cps);
		let after = run_program(codegen::compile_from_ir(&cps).expect("cps ir emit"));

		assert_eq!(
			(base.status.as_str(), base.stdout.as_str()),
			(after.status.as_str(), after.stdout.as_str()),
			"CPS transform changed behavior for `{name}`"
		);
		checked += 1;
	}

	assert!(
		checked > 100,
		"expected many fixtures, only checked {checked}"
	);
	assert!(
		total_transformed > 0,
		"expected the CPS pass to transform at least one async function"
	);
}

// The linear-chain showcase: `task-try-chain` has three poll-eligible functions
// (`add-one`, the captured `bump` closure, and `main` with three sequential
// awaits). All three transform, and the program still prints the same thing.
#[test]
fn try_chain_fixture_transforms_and_matches() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let dir = workspace.join("tests/run/task-try-chain");
	let compiler = compile_check(&dir).expect("task-try-chain compiles");
	let base_ir = ir::lower(&compiler).expect("lowers");

	let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

	let mut cps = base_ir.clone();
	ir::cps::cps_transform(&mut cps);
	assert!(
		poll_transformed_count(&cps) >= 3,
		"expected >=3 poll-transformed fns in task-try-chain, got {}",
		poll_transformed_count(&cps)
	);
	let after = run_program(codegen::compile_from_ir(&cps).expect("cps ir emit"));

	assert_eq!(base.status, after.status);
	assert_eq!(base.stdout, after.stdout);
	assert_eq!(base.status, "ok");
}

/// True if `f` awaits anywhere *inside* control flow (an `If`/`Switch`/`Match`/
/// `Loop` arm) — the shape the linear-chain cut couldn't handle and the CFG
/// transform exists to cover.
fn has_nested_await(f: &ir::Function) -> bool {
	fn block_await(b: &ir::Block) -> bool {
		b.0.iter().any(|s| match &s.kind {
			ir::StmtKind::Let(_, rv) | ir::StmtKind::Discard(rv) => {
				matches!(rv, ir::Rvalue::Await(_))
			}
			_ => child_await(s),
		})
	}
	fn child_await(s: &ir::Stmt) -> bool {
		match &s.kind {
			ir::StmtKind::If(_, t, e) => block_await(t) || block_await(e),
			ir::StmtKind::Switch { arms, default, .. } => {
				arms.iter().any(|(_, b)| block_await(b)) || block_await(default)
			}
			ir::StmtKind::Match { arms, .. } => arms.iter().any(|a| block_await(&a.body)),
			ir::StmtKind::Loop(b) => block_await(b),
			_ => false,
		}
	}
	f.body.0.iter().any(child_await)
}

// The nested-control-flow showcase: `task-combinators-concurrent` exercises
// `all`/`pool`, which call the stdlib `await-all`/`gather` — async functions
// with awaits nested inside `Match` arms (and, for `gather`, a top-level await
// followed by a nested-await `when`). The linear-chain cut left these
// Await-style; the CFG transform rewrites them. Every such fn here is
// defer-free, so all of them must transform, and the program still matches.
#[test]
fn nested_control_flow_transforms_and_matches() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let dir = workspace.join("tests/run/task-combinators-concurrent");
	let compiler = compile_check(&dir).expect("compiles");
	let base_ir = ir::lower(&compiler).expect("lowers");

	let nested: Vec<String> = base_ir
		.functions
		.iter()
		.filter(|f| f.is_async && has_nested_await(f))
		.map(|f| f.name.clone())
		.collect();
	assert!(
		nested.len() >= 2,
		"fixture should pull in >=2 nested-await async fns (await-all, gather), got {nested:?}"
	);

	let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

	let mut cps = base_ir.clone();
	ir::cps::cps_transform(&mut cps);
	for name in &nested {
		let f = cps.functions.iter().find(|f| &f.name == name).unwrap();
		assert!(
			f.poll_fn.is_some(),
			"expected nested-await async fn `{name}` to be poll-transformed"
		);
	}
	let after = run_program(codegen::compile_from_ir(&cps).expect("cps ir emit"));

	assert_eq!(base.status, after.status);
	assert_eq!(base.stdout, after.stdout);
	assert_eq!(base.status, "ok");
}

/// True if `f` schedules a `defer` anywhere (a `PushDefer`, possibly nested in
/// control flow) — the shape gated out of the acyclic cut until the cleanups
/// could be carried in the poll state.
fn has_defer(f: &ir::Function) -> bool {
	fn block_defer(b: &ir::Block) -> bool {
		b.0.iter().any(|s| match &s.kind {
			ir::StmtKind::PushDefer(_) => true,
			ir::StmtKind::If(_, t, e) => block_defer(t) || block_defer(e),
			ir::StmtKind::Switch { arms, default, .. } => {
				arms.iter().any(|(_, b)| block_defer(b)) || block_defer(default)
			}
			ir::StmtKind::Match { arms, .. } => arms.iter().any(|a| block_defer(&a.body)),
			ir::StmtKind::Loop(b) => block_defer(b),
			_ => false,
		})
	}
	block_defer(&f.body)
}

// The `defer`-across-suspension showcase. `defer` was gated out of the acyclic
// cut because the scheduled cleanups must survive each suspension; now they ride
// in the poll state (`__defers`) and the driver runs them LIFO on every exit
// edge. These three fixtures cover all three edges: `task-defer` completes
// normally (defers run on the `ready` path), `task-fail` propagates a failure
// (defers run in the err-walk as it unwinds through nested async fns), and
// `scope-race` cancels a loser mid-await (defers run in `reap_fiber`). Each has
// >=1 defer-bearing async fn, all loop-free, so all transform — and behavior
// must stay byte-identical to the Await-style run.
#[test]
fn defer_across_suspension_transforms_and_matches() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	for fixture in ["task-defer", "task-fail", "scope-race"] {
		let dir = workspace.join("tests/run").join(fixture);
		let compiler = compile_check(&dir).unwrap_or_else(|| panic!("{fixture} compiles"));
		let base_ir = ir::lower(&compiler).expect("lowers");

		let deferred: Vec<String> = base_ir
			.functions
			.iter()
			.filter(|f| f.is_async && has_defer(f))
			.map(|f| f.name.clone())
			.collect();
		assert!(
			!deferred.is_empty(),
			"[{fixture}] should pull in >=1 defer-bearing async fn, found none"
		);

		let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

		let mut cps = base_ir.clone();
		ir::cps::cps_transform(&mut cps);
		for name in &deferred {
			let f = cps.functions.iter().find(|f| &f.name == name).unwrap();
			assert!(
				f.poll_fn.is_some(),
				"[{fixture}] expected defer-bearing async fn `{name}` to be poll-transformed"
			);
		}
		let after = run_program(codegen::compile_from_ir(&cps).expect("cps ir emit"));

		assert_eq!(
			(base.status.as_str(), base.stdout.as_str()),
			(after.status.as_str(), after.stdout.as_str()),
			"[{fixture}] CPS defer transform changed behavior"
		);
	}
}

/// True if `f` contains a `Loop` *with an `Await` inside it* — an `await` in a
/// `while` body. The flattener turns the loop's back-edge into the dispatch loop,
/// splitting the loop at the suspension and threading the loop-carried vars.
fn has_loop_with_await(f: &ir::Function) -> bool {
	fn block(b: &ir::Block) -> bool {
		b.0.iter().any(stmt)
	}
	fn block_await(b: &ir::Block) -> bool {
		b.0.iter().any(|s| match &s.kind {
			ir::StmtKind::Let(_, rv) | ir::StmtKind::Discard(rv) => matches!(rv, ir::Rvalue::Await(_)),
			ir::StmtKind::If(_, t, e) => block_await(t) || block_await(e),
			ir::StmtKind::Switch { arms, default, .. } => {
				arms.iter().any(|(_, b)| block_await(b)) || block_await(default)
			}
			ir::StmtKind::Match { arms, .. } => arms.iter().any(|a| block_await(&a.body)),
			ir::StmtKind::Loop(b) => block_await(b),
			_ => false,
		})
	}
	fn stmt(s: &ir::Stmt) -> bool {
		match &s.kind {
			ir::StmtKind::Loop(b) => block_await(b),
			ir::StmtKind::If(_, t, e) => block(t) || block(e),
			ir::StmtKind::Switch { arms, default, .. } => {
				arms.iter().any(|(_, b)| block(b)) || block(default)
			}
			ir::StmtKind::Match { arms, .. } => arms.iter().any(|a| block(&a.body)),
			_ => false,
		}
	}
	block(&f.body)
}

// The `await`-in-loop showcase. `task-loop` sums a list via a `while` that
// `await`s `task.sleep` each iteration — a `Loop` with an `Await` inside, which
// was gated out of CPS entirely (a source back-edge). The flattener now turns
// the loop into the dispatch loop's back-edge, splits at the suspension, and
// threads the loop-carried `ref`s + pattern binds through the state. `main` must
// transform, and the program still prints the same sum.
#[test]
fn loop_with_await_transforms_and_matches() {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let dir = workspace.join("tests/run/task-loop");
	let compiler = compile_check(&dir).expect("task-loop compiles");
	let base_ir = ir::lower(&compiler).expect("lowers");

	let looping: Vec<String> = base_ir
		.functions
		.iter()
		.filter(|f| f.is_async && has_loop_with_await(f))
		.map(|f| f.name.clone())
		.collect();
	assert!(
		!looping.is_empty(),
		"task-loop should pull in >=1 async fn that awaits inside a loop, found none"
	);

	let base = run_program(codegen::compile_from_ir(&base_ir).expect("ir emit"));

	let mut cps = base_ir.clone();
	ir::cps::cps_transform(&mut cps);
	for name in &looping {
		let f = cps.functions.iter().find(|f| &f.name == name).unwrap();
		assert!(
			f.poll_fn.is_some(),
			"expected await-in-loop async fn `{name}` to be poll-transformed"
		);
	}
	let after = run_program(codegen::compile_from_ir(&cps).expect("cps ir emit"));

	assert_eq!(base.status, after.status);
	assert_eq!(base.stdout, after.stdout);
	assert_eq!(base.status, "ok");
	assert_eq!(base.stdout.trim(), "10");
}
