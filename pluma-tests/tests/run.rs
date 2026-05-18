// One #[test] per `tests/run/<name>/main.pa` fixture. Compiles + runs the
// fixture in-process via the bytecode VM, capturing `print` output through
// the VM's configurable StdoutSink. Snapshot lives in `run.snap` next to the
// fixture.

use compiler::{Compiler, Diagnostic};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

datatest_stable::harness!(
	run_fixture,
	concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/run"),
	r"main\.pa$"
);

fn run_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let relative = path.strip_prefix(workspace).unwrap_or(path);

	let stdout_buf = Rc::new(RefCell::new(Vec::<u8>::new()));
	let result = (|| -> Result<(), RunError> {
		let mut compiler = Compiler::from_entry_path(relative.to_str().unwrap().to_string())
			.map_err(RunError::Diagnostics)?;
		vm::stdlib::register_compiler(&mut compiler);
		compiler.check().map_err(RunError::Diagnostics)?;
		let program = codegen::compile(&compiler).map_err(RunError::Runtime)?;
		let mut vm_instance =
			vm::VM::new(program).with_stdout(vm::StdoutSink::Buffer(stdout_buf.clone()));
		vm_instance.run().map_err(|e| RunError::Runtime(e.message))?;
		Ok(())
	})();

	let stdout = String::from_utf8_lossy(&stdout_buf.borrow()).to_string();
	let (status, stderr) = match result {
		Ok(()) => ("ok".to_string(), String::new()),
		Err(RunError::Diagnostics(d)) => ("compile error".to_string(), format_diagnostics(&d)),
		Err(RunError::Runtime(msg)) => ("runtime error".to_string(), format!("{}\n", msg)),
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
