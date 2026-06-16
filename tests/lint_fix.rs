// The lint autofix suite. One #[test] per `tests/lint/<name>/main.pa` fixture —
// the same corpus the `lint` suite audits for warnings, here run through the
// autofixer. Each fixture is fixed exactly as `pluma lint --fix` does it
// (`linter::fix_source` then a reformat), and the result is snapshotted to
// `fix.snap` next to the fixture. A fixture with no autofixable lint renders a
// loud sentinel, so a rule that gains or loses a fix surfaces as a snapshot diff.
//
// Two round-trip invariants are asserted beyond the snapshot:
//   - the fixed source still parses (the reformat would fail otherwise), and
//   - the fix is a fixpoint — re-running the fixer on the fixed output finds
//     nothing more to do. A fix that re-triggers its own lint would loop under
//     `--fix`, so this guards against that.

use std::fs;
use std::path::Path;

datatest_stable::harness! {
	{ test = lint_fix_fixture, root = concat!(env!("CARGO_MANIFEST_DIR"), "/lint"), pattern = r"main\.pa$" },
}

fn lint_fix_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	let bytes = fs::read(path)?;

	let output = match linter::fix_source(&bytes) {
		// Reformat the rewrite, mirroring `pluma lint --fix`: the edits can leave
		// non-canonical whitespace (a freshly wrapped `using` block lands on one
		// line) that the formatter then lays out.
		Ok(Some(fixed)) => {
			let formatted = formatter::format_source(fixed.as_bytes()).map_err(|diagnostics| {
				format!(
					"autofix for {} produced unparseable output:\n{}\n--- diagnostics ---\n{}",
					path.display(),
					fixed,
					diagnostics
						.iter()
						.map(|d| d.message.clone())
						.collect::<Vec<_>>()
						.join("; ")
				)
			})?;

			// The fixer must reach a fixpoint in one pass: applying it again to
			// the formatted result finds nothing left to fix.
			match linter::fix_source(formatted.as_bytes()) {
				Ok(None) => {}
				Ok(Some(_)) => {
					return Err(
						format!(
							"autofix for {} is not a fixpoint — re-running it changes the output again:\n{}",
							path.display(),
							formatted
						)
						.into(),
					);
				}
				Err(_) => return Err(format!("re-fixing {} failed to parse", path.display()).into()),
			}

			formatted
		}
		// No autofixable lint in this fixture.
		Ok(None) => "(no autofixes)\n".to_string(),
		Err(diagnostics) => {
			return Err(
				format!(
					"fix_source failed to parse {}: {}",
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

	insta::with_settings!({
		snapshot_path => fixture_dir,
		prepend_module_to_snapshot => false,
	}, {
		insta::assert_snapshot!("fix", output);
	});

	Ok(())
}
