use crate::printing::*;
use std::path::PathBuf;

/// `pluma lint <paths…>` — parse each module and report lint warnings, or with
/// `--fix` apply the autofixable ones in place. Reports exit non-zero if any
/// lint fires (so CI can gate on a clean lint) or if any file can't be read.
/// Files that don't parse are skipped with a note, mirroring `pluma format` — a
/// lint sweep may include intentionally-broken fixtures.
pub(crate) fn lint_command(fix: bool, paths: Vec<String>) {
	if paths.is_empty() {
		print_error("No path given. Expected a file path or `-` for stdin.");
		std::process::exit(1);
	}

	if fix {
		fix_command(paths);
		return;
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

/// `pluma lint --fix` — apply autofixes in place. For each file, rewrite it with
/// the available fixes and then reformat (the rewrites collapse `let _ =` /
/// lambda wrappers and can leave non-canonical whitespace). Unparseable files
/// are skipped; files with no fixes are left untouched. With `-`, the rewritten
/// module is written to stdout.
fn fix_command(paths: Vec<String>) {
	let mut fixed_count = 0usize;

	for path in &paths {
		if path == "-" {
			let mut input = Vec::new();
			if let Err(err) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut input) {
				print_error(format!("Failed to read stdin: {}", err));
				std::process::exit(1);
			}
			let fixed = match linter::fix_source(&input) {
				Ok(Some(text)) => text,
				Ok(None) => String::from_utf8_lossy(&input).into_owned(),
				Err(diagnostics) => {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}
			};
			print!("{}", reformat(fixed.as_bytes()));
			continue;
		}

		let bytes = match std::fs::read(path) {
			Ok(b) => b,
			Err(err) => {
				print_error(format!("Could not read `{}`: {}", path, err));
				std::process::exit(1);
			}
		};

		match linter::fix_source(&bytes) {
			Ok(Some(fixed)) => {
				let out = reformat(fixed.as_bytes());
				if let Err(err) = std::fs::write(path, out.as_bytes()) {
					print_error(format!("Could not write `{}`: {}", path, err));
					std::process::exit(1);
				}
				fixed_count += 1;
				eprintln!("fixed {}", path);
			}
			// No fixes — leave the file (and its formatting) untouched.
			Ok(None) => {}
			Err(_diagnostics) => {
				eprintln!("skipping {} (parse error)", path);
			}
		}
	}

	eprintln!(
		"{} file{} fixed",
		fixed_count,
		if fixed_count == 1 { "" } else { "s" }
	);
}

/// Reformat fixed source, falling back to the unformatted text if the rewrite
/// somehow doesn't parse (it always should — fixes preserve well-formedness).
fn reformat(source: &[u8]) -> String {
	match formatter::format_source(source) {
		Ok(formatted) => formatted,
		Err(_) => String::from_utf8_lossy(source).into_owned(),
	}
}
