// The error-message suite. One #[test] per `tests/errors/<name>/main.pa`
// fixture, each a minimal program that is *supposed* to fail. We run the
// compiler frontend and snapshot the diagnostics rendered exactly as the CLI
// renders them (via `compiler::render_diagnostics`), minus ANSI color — so this
// corpus is a direct audit of error-message quality (codes, carets, help,
// notes, suggestions). Snapshots live in `errors.snap` next to the fixture.
//
// A fixture that compiles clean renders a loud sentinel instead of diagnostics,
// so an error path that quietly stops firing surfaces as a snapshot diff.

use compiler::{Compiler, Diagnostic, Palette, render_diagnostics};
use std::fs;
use std::path::Path;

datatest_stable::harness! {
	{ test = errors_fixture, root = concat!(env!("CARGO_MANIFEST_DIR"), "/errors"), pattern = r"main(\.test)?\.pa$" },
}

fn errors_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	// Anchor cwd at the workspace root so the renderer trims it off the
	// displayed path (`tests/errors/<name>/main.pa`). Idempotent across tests.
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let relative = path.strip_prefix(workspace).unwrap_or(path);

	let diagnostics: Vec<Diagnostic> =
		match Compiler::from_entry_path(relative.to_str().unwrap().to_string()) {
			Ok(mut compiler) => match compiler.check() {
				Ok(()) => Vec::new(),
				Err(diagnostics) => diagnostics,
			},
			Err(diagnostics) => diagnostics,
		};

	let output = if diagnostics.is_empty() {
		"(no diagnostics — fixture unexpectedly compiled)\n".to_string()
	} else {
		render_diagnostics(
			&diagnostics,
			|p: &Path| fs::read_to_string(p).ok(),
			&Palette::plain(),
		)
	};

	insta::with_settings!({
		snapshot_path => fixture_dir,
		prepend_module_to_snapshot => false,
	}, {
		insta::assert_snapshot!("errors", output);
	});

	Ok(())
}
