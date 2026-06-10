use crate::printing::*;
use std::path::PathBuf;

/// `pluma lint <paths…>` — parse each module and report lint warnings. Reports
/// only; never rewrites. Exits non-zero if any lint fires (so CI can gate on a
/// clean lint) or if any file can't be read. Files that don't parse are skipped
/// with a note, mirroring `pluma format` — a lint sweep may include
/// intentionally-broken fixtures.
pub(crate) fn lint_command(paths: Vec<String>) {
	if paths.is_empty() {
		print_error("No path given. Expected a file path or `-` for stdin.");
		std::process::exit(1);
	}

	let mut any_warnings = false;

	for path in &paths {
		let result = if path == "-" {
			let mut input = Vec::new();
			if let Err(err) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut input) {
				print_error(format!("Failed to read stdin: {}", err));
				std::process::exit(1);
			}
			linter::lint_source(&input)
		} else {
			let bytes = match std::fs::read(path) {
				Ok(b) => b,
				Err(err) => {
					print_error(format!("Could not read `{}`: {}", path, err));
					std::process::exit(1);
				}
			};
			linter::lint_path(&PathBuf::from(path), &bytes)
		};

		match result {
			Ok(warnings) => {
				if !warnings.is_empty() {
					any_warnings = true;
					print_diagnostics(warnings);
				}
			}
			Err(_diagnostics) => {
				// Skip unparseable files rather than aborting — the user may be
				// linting a batch that includes intentionally-broken fixtures.
				eprintln!("skipping {} (parse error)", path);
			}
		}
	}

	if any_warnings {
		std::process::exit(1);
	}
}
