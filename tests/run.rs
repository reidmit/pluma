// One #[test] per execution fixture. Compiles + runs the fixture in-process via
// the bytecode VM, capturing `print` output through the VM's configurable
// OutputSink. Snapshot lives in `run.snap` next to the fixture. Two roots, same
// harness: `run/` is the happy-path corpus (status `ok`) and `run-fail/` holds
// programs that compile but fail at runtime (status `runtime error`). Compile-
// error cases live in `tests/analyze/` (the frontend suite), not here.

use compiler::{Compiler, Diagnostic};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

datatest_stable::harness! {
	{ test = run_fixture, root = concat!(env!("CARGO_MANIFEST_DIR"), "/run"), pattern = r"main\.pa$" },
	{ test = run_fixture, root = concat!(env!("CARGO_MANIFEST_DIR"), "/run-fail"), pattern = r"main\.pa$" },
}

fn run_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	// See the analogous comment in analyze.rs: anchor cwd at the workspace
	// root (one level up from this crate) so portable relative paths show
	// up in fixture output.
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let relative = path.strip_prefix(workspace).unwrap_or(path);

	let stdout_buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let stderr_buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	// If a fixture has stdin.txt next to main.pa, feed its bytes as the
	// program's stdin. Otherwise stdin is empty (any read returns EOF).
	let stdin_bytes = std::fs::read(fixture_dir.join("stdin.txt")).unwrap_or_default();
	let stdin_buf = Rc::new(RefCell::new(stdin_bytes));
	let result = (|| -> Result<(), RunError> {
		let mut compiler = Compiler::from_entry_path(relative.to_str().unwrap().to_string())
			.map_err(RunError::Diagnostics)?;
		vm::stdlib::register_compiler(&mut compiler);
		compiler.check().map_err(RunError::Diagnostics)?;
		let mut ir_program = ir::lower(&compiler).map_err(RunError::Runtime)?;
		ir::optimize(&mut ir_program);
		let program = codegen::compile_from_ir(&ir_program).map_err(RunError::Runtime)?;
		let mut vm_instance = vm::VM::new(program)
			.with_stdout(vm::OutputSink::Buffer(stdout_buf.clone()))
			.with_stderr(vm::OutputSink::Buffer(stderr_buf.clone()))
			.with_stdin(vm::InputSource::Buffer(stdin_buf.clone()));
		vm_instance
			.run()
			.map_err(|e| RunError::Runtime(e.message))?;
		Ok(())
	})();

	let stdout = String::from_utf8_lossy(&stdout_buf.borrow()).to_string();
	let mut stderr = String::from_utf8_lossy(&stderr_buf.borrow()).to_string();
	let status = match result {
		Ok(()) => "ok".to_string(),
		Err(RunError::Diagnostics(d)) => {
			stderr.push_str(&format_diagnostics(&d));
			"compile error".to_string()
		}
		Err(RunError::Runtime(msg)) => {
			stderr.push_str(&format!("{}\n", msg));
			"runtime error".to_string()
		}
	};

	let combined = format!(
		"== status ==\n{}\n== stdout ==\n{}== stderr ==\n{}",
		status, stdout, stderr
	);

	insta::with_settings!({
		snapshot_path => fixture_dir,
		prepend_module_to_snapshot => false,
	}, {
		insta::assert_snapshot!("run", combined);
	});

	Ok(())
}

enum RunError {
	Diagnostics(Vec<Diagnostic>),
	Runtime(String),
}

fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
	use std::fmt::Write;
	let mut out = String::new();
	for d in diagnostics {
		let kind = if d.is_error() { "error" } else { "warning" };
		writeln!(&mut out, "{}: {}", kind, d.message).unwrap();
	}
	out
}
