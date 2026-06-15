use compiler::{Compiler, Diagnostic, Module, ModuleExports};
use std::collections::HashMap;
use std::path::Path;

thread_local! {
	// The analyzed stdlib export tables, computed once per worker thread and
	// reused for the life of the LSP process. The stdlib source is baked into
	// the binary and never changes, so its analysis is invariant — seeding it
	// into every per-keystroke `Compiler` skips re-parsing and re-analyzing
	// ~all of `std/*` on each edit (the dominant fixed cost of analyzing a
	// single user file). `thread_local` rather than a `static`: the export
	// tables carry trait-default AST bodies whose dispatch cells are `Rc`
	// (neither `Send` nor `Sync`), so they can't be shared across threads.
	// Reusing them across sequential analyses on one thread is sound for the
	// same reason it already is within a single compile — an importer clones a
	// default's body and assigns *fresh* dispatch cells during analysis rather
	// than mutating the shared template.
	static STDLIB_EXPORTS: HashMap<String, ModuleExports> = Compiler::stdlib_export_table();
}

pub struct AnalysisResult {
	// The analyzed entry module. Present even when diagnostics is non-empty
	// — the analyzer attaches the inferred types it could resolve, so hover
	// still has something useful to show on partial failures.
	pub module: Option<Module>,
	pub diagnostics: Vec<Diagnostic>,
}

// Run the full compiler pipeline (parse + analyze) against an in-memory
// document, with imports resolved from disk relative to the document's
// containing project. Returns the analyzed entry module and any
// diagnostics produced along the way.
pub fn analyze_document(path: &Path, source: Vec<u8>) -> AnalysisResult {
	let entry_path_str = match path.to_str() {
		Some(s) => s.to_string(),
		None => {
			return AnalysisResult {
				module: None,
				diagnostics: vec![Diagnostic::error("document path is not valid UTF-8")],
			};
		}
	};

	let mut compiler = match Compiler::from_entry_path(entry_path_str) {
		Ok(c) => c,
		Err(diagnostics) => {
			return AnalysisResult {
				module: None,
				diagnostics,
			};
		}
	};

	let entry_name = compiler.entry_modules.first().cloned().unwrap_or_default();
	compiler.set_module_source(entry_name.clone(), source);

	// Reuse the per-thread stdlib analysis instead of re-doing it per edit.
	STDLIB_EXPORTS.with(|exports| compiler.seed_exports(exports));

	let diagnostics = compiler.check().err().unwrap_or_default();
	let module = compiler.modules.remove(&entry_name);

	AnalysisResult {
		module,
		diagnostics,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;
	use std::sync::atomic::{AtomicU32, Ordering};

	// Materialize `src` to a unique temp `main.pa` and analyze it, returning the
	// rendered diagnostics. A flag picks whether to seed the stdlib export cache
	// — so a test can compare the fast seeded path against a cold from-scratch
	// analysis of the same source.
	fn diagnostics_for(src: &str, seed: bool) -> Vec<String> {
		static COUNTER: AtomicU32 = AtomicU32::new(0);
		let n = COUNTER.fetch_add(1, Ordering::Relaxed);
		let mut dir: PathBuf = std::env::temp_dir();
		dir.push(format!("pluma-seed-{}-{}", std::process::id(), n));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("main.pa");
		std::fs::write(&path, src).unwrap();

		let mut compiler = match Compiler::from_entry_path(path.to_str().unwrap().to_string()) {
			Ok(c) => c,
			Err(_) => panic!("compiler init failed"),
		};
		let entry = compiler.entry_modules.first().cloned().unwrap();
		compiler.set_module_source(entry, src.as_bytes().to_vec());
		if seed {
			STDLIB_EXPORTS.with(|exports| compiler.seed_exports(exports));
		}
		let diags = compiler.check().err().unwrap_or_default();
		std::fs::remove_dir_all(&dir).ok();
		// Compare on the user-visible content only — `module_path` is the temp
		// file, which differs per call by construction.
		diags
			.iter()
			.map(|d| {
				let range = d
					.range
					.map(|r| {
						format!(
							"{}:{}-{}:{}",
							r.start.line, r.start.col, r.end.line, r.end.col
						)
					})
					.unwrap_or_default();
				format!("{:?} {:?} {} {:?}", d.code, d.message, range, d.notes)
			})
			.collect()
	}

	// The seeded fast path must be observationally identical to a cold analysis:
	// seeding only short-circuits *re-deriving* the immutable stdlib exports, so
	// every diagnostic the user sees must match byte-for-byte. Covers a clean
	// file, a type error, and an unknown-name error across several stdlib deps.
	#[test]
	fn seeded_analysis_matches_cold_analysis() {
		let cases = [
			// Clean: exercises list/string/dict imports + inference.
			"use std/list\nuse std/string\nuse std/dict\n\ndef main = fun {\n\tlet xs = [1, 2, 3]\n\tlist.length xs\n}\n",
			// Type error: int + float with no implicit promotion.
			"def main = fun {\n\t2 + 3.5\n}\n",
			// Unknown name in an imported module.
			"use std/list\n\ndef main = fun {\n\tlist.nonexistent [1]\n}\n",
		];
		for src in cases {
			assert_eq!(
				diagnostics_for(src, true),
				diagnostics_for(src, false),
				"seeded and cold diagnostics diverged for:\n{src}"
			);
		}
	}

	// Not a correctness assert — prints the per-analysis cost with and without
	// the stdlib seed so the speedup is visible under `--nocapture`. Ignored by
	// default (timing is machine-dependent and not a regression gate).
	#[test]
	#[ignore]
	fn seed_speedup_timing() {
		let src = "use std/list\nuse std/string\nuse std/dict\nuse std/json\nuse std/math\n\ndef main = fun {\n\tlet xs = [1, 2, 3]\n\tlist.length xs\n}\n";
		// Warm the per-thread cache first (its one-time build isn't per-keystroke).
		STDLIB_EXPORTS.with(|_| {});
		let reps = 50;

		let t = std::time::Instant::now();
		for _ in 0..reps {
			let _ = diagnostics_for(src, false);
		}
		let cold = t.elapsed() / reps;

		let t = std::time::Instant::now();
		for _ in 0..reps {
			let _ = diagnostics_for(src, true);
		}
		let seeded = t.elapsed() / reps;

		eprintln!("cold (re-analyze stdlib): {cold:?}/analysis");
		eprintln!("seeded (reuse stdlib):    {seeded:?}/analysis");
	}
}
