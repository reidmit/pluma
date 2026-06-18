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

	let paths = crate::commands::expand_paths(paths);

	if fix {
		fix_command(paths);
		return;
	}

	let mut total_issues = 0usize;
	let mut file_count = 0usize;

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
				file_count += 1;
				total_issues += warnings.len();
				if !warnings.is_empty() {
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

	eprintln!("{}", summary(total_issues, file_count, None));

	if total_issues > 0 {
		std::process::exit(1);
	}
}

/// `pluma lint --fix` — apply autofixes in place. For each file, rewrite it with
/// the available fixes and then reformat (the rewrites collapse `let _ =` /
/// lambda wrappers and can leave non-canonical whitespace). Unparseable files
/// are skipped; files with no fixes are left untouched. With `-`, the rewritten
/// module is written to stdout.
fn fix_command(paths: Vec<String>) {
	let mut total_issues = 0usize;
	let mut total_fixed = 0usize;
	let mut file_count = 0usize;

	for path in &paths {
		let bytes = if path == "-" {
			let mut input = Vec::new();
			if let Err(err) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut input) {
				print_error(format!("Failed to read stdin: {}", err));
				std::process::exit(1);
			}
			input
		} else {
			match std::fs::read(path) {
				Ok(b) => b,
				Err(err) => {
					print_error(format!("Could not read `{}`: {}", path, err));
					std::process::exit(1);
				}
			}
		};

		// Count this file's issues (and how many are autofixable) before applying
		// the rewrite, so the summary can report found-vs-fixed.
		let findings = match linter::lint_findings(&bytes) {
			Ok(f) => f,
			Err(diagnostics) => {
				if path == "-" {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}
				eprintln!("skipping {} (parse error)", path);
				continue;
			}
		};
		file_count += 1;
		total_issues += findings.len();
		total_fixed += findings.iter().filter(|f| !f.fixes.is_empty()).count();

		// `lint_findings` already established the source parses, so `fix_source`
		// won't hit the error arm here.
		let fixed = match linter::fix_source(&bytes) {
			Ok(fixed) => fixed,
			Err(diagnostics) => {
				print_diagnostics(diagnostics);
				std::process::exit(1);
			}
		};

		if path == "-" {
			let text = fixed.unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
			print!("{}", reformat(text.as_bytes()));
			continue;
		}

		// No fixes — leave the file (and its formatting) untouched.
		if let Some(fixed) = fixed {
			let out = reformat(fixed.as_bytes());
			if let Err(err) = std::fs::write(path, out.as_bytes()) {
				print_error(format!("Could not write `{}`: {}", path, err));
				std::process::exit(1);
			}
			eprintln!("fixed {}", path);
		}
	}

	eprintln!("{}", summary(total_issues, file_count, Some(total_fixed)));
}

/// The trailing summary line, e.g. `found 3 issues in 2 files` or, in `--fix`
/// mode, `found 3 issues in 2 files (fixed 2)`.
fn summary(issues: usize, files: usize, fixed: Option<usize>) -> String {
	let issues = format!("{} issue{}", issues, if issues == 1 { "" } else { "s" });
	let files = format!("{} file{}", files, if files == 1 { "" } else { "s" });
	match fixed {
		Some(fixed) => format!("found {} in {} (fixed {})", issues, files, fixed),
		None => format!("found {} in {}", issues, files),
	}
}

/// Reformat fixed source, falling back to the unformatted text if the rewrite
/// somehow doesn't parse (it always should — fixes preserve well-formedness).
fn reformat(source: &[u8]) -> String {
	match formatter::format_source(source) {
		Ok(formatted) => formatted,
		Err(_) => String::from_utf8_lossy(source).into_owned(),
	}
}
