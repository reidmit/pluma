// The lint suite. One #[test] per `tests/lint/<name>/main.pa` fixture. Each
// fixture is run through the linter and its warnings are rendered exactly as the
// CLI renders them (via `compiler::render_diagnostics`, minus ANSI color), then
// snapshotted to `lint.snap` next to the fixture. The corpus is a direct audit
// of which lints fire, where, and with what message/help.
//
// A fixture that produces no lints renders a loud sentinel instead, so a rule
// that quietly stops firing surfaces as a snapshot diff.

use compiler::{Palette, render_diagnostics};
use std::fs;
use std::path::Path;

datatest_stable::harness! {
	{ test = lint_fixture, root = concat!(env!("CARGO_MANIFEST_DIR"), "/lint"), pattern = r"main\.pa$" },
}

fn lint_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	// Anchor cwd at the workspace root so the renderer trims it off the
	// displayed path (`tests/lint/<name>/main.pa`). Idempotent across tests.
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
	let _ = std::env::set_current_dir(workspace);
	let relative = path.strip_prefix(workspace).unwrap_or(path);

	let bytes = fs::read(path)?;
	let warnings = match linter::lint_path(relative, &bytes) {
		Ok(w) => w,
		Err(diagnostics) => {
			return Err(
				format!(
					"lint_path failed to parse {}: {}",
					path.display(),
					diagnostics
						.iter()
						.map(|d| d.message.clone())
						.collect::<Vec<_>>()
						.join("; ")
				)
				.into(),
			);
		}
	};

	let output = if warnings.is_empty() {
		"(no lints)\n".to_string()
	} else {
		render_diagnostics(
			&warnings,
			|p: &Path| fs::read_to_string(p).ok(),
			&Palette::plain(),
			None,
		)
	};

	insta::with_settings!({
		snapshot_path => fixture_dir,
		prepend_module_to_snapshot => false,
	}, {
		insta::assert_snapshot!("lint", output);
	});

	Ok(())
}
