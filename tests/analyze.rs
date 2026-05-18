// One #[test] per `tests/analyze/<name>/main.pa` fixture. Each test loads the
// fixture in-process, runs the compiler frontend, and snapshots the formatted
// result (Debug-dump of the typed Module on success, formatted diagnostics on
// failure). Snapshots live in `analyze.snap` next to the fixture.

use compiler::{Compiler, Diagnostic};
use std::path::Path;

datatest_stable::harness!(
	analyze_fixture,
	concat!(env!("CARGO_MANIFEST_DIR"), "/analyze"),
	r"main\.pa$"
);

fn analyze_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	// This crate lives at <workspace>/tests/, so the workspace root is one
	// level up. Anchoring cwd here lets Module's Debug impl trim it off the
	// rendered path (`tests/analyze/<name>/main.pa`). Idempotent (all tests
	// use the same value), so the shared cwd doesn't race.
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let relative = path.strip_prefix(workspace).unwrap_or(path);

	let result = (|| -> Result<String, Vec<Diagnostic>> {
		let mut compiler = Compiler::from_entry_path(relative.to_str().unwrap().to_string())?;
		vm::stdlib::register_compiler(&mut compiler);
		let module = compiler.check()?;
		Ok(format!("{:#?}", module))
	})();

	let output = match result {
		Ok(s) => s,
		Err(diagnostics) => format_diagnostics(&diagnostics),
	};

	insta::with_settings!({
		snapshot_path => fixture_dir,
		prepend_module_to_snapshot => false,
	}, {
		insta::assert_snapshot!("analyze", output);
	});

	Ok(())
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
