// One #[test] per `tests/analyze/<name>/main.pa` fixture. Each test loads the
// fixture in-process, runs the compiler frontend, and snapshots the formatted
// result (Debug-dump of the typed Module on success, formatted diagnostics on
// failure). Snapshots live in `analyze.snap` next to the fixture.

use compiler::{Compiler, Diagnostic};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

datatest_stable::harness! {
	{ test = analyze_fixture, root = concat!(env!("CARGO_MANIFEST_DIR"), "/analyze"), pattern = r"main(\.test)?\.pa$" },
}

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
		compiler.check()?;
		let entry_name = compiler.entry_modules.first().cloned().unwrap_or_default();
		let module = compiler.modules.get(&entry_name).unwrap();
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

// Renders each diagnostic as message + (when available) a code excerpt with
// a caret marker. Mirrors what the CLI prints to stderr, minus ANSI color
// codes — enough to verify both the wording and the location.
fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
	use std::fmt::Write;
	let mut out = String::new();
	let mut file_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();

	for (i, d) in diagnostics.iter().enumerate() {
		if i > 0 {
			writeln!(&mut out).unwrap();
		}
		let kind = if d.is_error() { "error" } else { "warning" };
		writeln!(&mut out, "{}: {}", kind, d.message).unwrap();

		let (Some(range), Some(path)) = (d.range, d.module_path.as_ref()) else {
			continue;
		};

		let lines = file_cache.entry(path.clone()).or_insert_with(|| {
			std::fs::read_to_string(path)
				.map(|s| s.lines().map(|l| l.to_string()).collect())
				.unwrap_or_default()
		});

		// Render the start line with a caret. Multi-line ranges show only
		// the start line; the caret extends to end-of-line. Tabs are kept
		// as-is so the caret column matches the source byte offset.
		let line_idx = range.start.line;
		let Some(text) = lines.get(line_idx) else {
			continue;
		};
		let line_num = line_idx + 1;
		let prefix = format!("> {} | ", line_num);
		writeln!(&mut out, "{}{}", prefix, text).unwrap();

		let caret_len = if range.start.line == range.end.line {
			range.end.col.saturating_sub(range.start.col).max(1)
		} else {
			text.len().saturating_sub(range.start.col).max(1)
		};
		let pad = " ".repeat(prefix.len() + range.start.col);
		let carets = "^".repeat(caret_len);
		writeln!(&mut out, "{}{}", pad, carets).unwrap();
	}
	out
}
