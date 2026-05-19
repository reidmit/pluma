// One #[test] per `tests/format/<name>/main.pa` fixture. Each test reads the
// fixture, formats it, asserts the formatter is idempotent (formatting twice
// produces the same output), and snapshots the formatted output to
// `format.snap` next to the fixture.

use std::path::Path;

datatest_stable::harness!(
	format_fixture,
	concat!(env!("CARGO_MANIFEST_DIR"), "/format"),
	r"main\.pa$"
);

fn format_fixture(path: &Path) -> datatest_stable::Result<()> {
	let fixture_dir = path.parent().unwrap();
	let bytes = std::fs::read(path)?;

	let once = match formatter::format_source(&bytes) {
		Ok(s) => s,
		Err(diagnostics) => {
			return Err(
				format!(
					"format_source failed for {}: {}",
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

	let twice = formatter::format_source(once.as_bytes())
		.map_err(|_| "formatter produced unparseable output")?;

	if once != twice {
		return Err(
			format!(
				"formatter not idempotent for {}:\n--- once ---\n{}--- twice ---\n{}",
				path.display(),
				once,
				twice
			)
			.into(),
		);
	}

	insta::with_settings!({
		snapshot_path => fixture_dir,
		prepend_module_to_snapshot => false,
	}, {
		insta::assert_snapshot!("format", once);
	});

	Ok(())
}
