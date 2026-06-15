use compiler::{Compiler, Diagnostic, Module, ModuleCache, ModuleExports};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

thread_local! {
	// Per-thread incremental export cache: analyzed exports of the user
	// modules seen so far, keyed by name and gated by source hash. Swapped
	// into each per-keystroke `Compiler` so editing one file doesn't force
	// re-analysis of the unchanged modules it imports (only the edited entry
	// and anything whose dependencies changed are reanalyzed). Cleared
	// implicitly as hashes stop matching; entries for clean modules persist.
	static MODULE_CACHE: RefCell<ModuleCache> = RefCell::new(HashMap::new());
}

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

	// Lend the per-thread incremental cache to this compile, then reclaim it
	// (refreshed) so the next keystroke can skip unchanged user modules.
	compiler.enable_incremental(MODULE_CACHE.with(|c| std::mem::take(&mut *c.borrow_mut())));

	let diagnostics = compiler.check().err().unwrap_or_default();
	let module = compiler.modules.remove(&entry_name);

	let cache = compiler.take_incremental_cache();
	MODULE_CACHE.with(|c| *c.borrow_mut() = cache);

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

	// Render diagnostics to their user-visible content only — `module_path` is
	// a temp file that differs per call by construction, so it's excluded.
	fn render(diags: &[Diagnostic]) -> Vec<String> {
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

	fn unique_dir(tag: &str) -> PathBuf {
		static COUNTER: AtomicU32 = AtomicU32::new(0);
		let n = COUNTER.fetch_add(1, Ordering::Relaxed);
		let mut dir = std::env::temp_dir();
		dir.push(format!("pluma-{}-{}-{}", tag, std::process::id(), n));
		std::fs::create_dir_all(&dir).unwrap();
		dir
	}

	// Materialize `src` to a unique temp `main.pa` and analyze it, returning the
	// rendered diagnostics. A flag picks whether to seed the stdlib export cache
	// — so a test can compare the fast seeded path against a cold from-scratch
	// analysis of the same source.
	fn diagnostics_for(src: &str, seed: bool) -> Vec<String> {
		let dir = unique_dir("seed");
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
		render(&diags)
	}

	// One incremental analysis pass over a project dir: re-read `main.pa` from
	// `main_src` (the unsaved buffer), reusing what `cache` permits. Returns the
	// rendered diagnostics, the set of modules actually (re)analyzed, and the
	// refreshed cache to thread into the next pass.
	fn incr_pass(
		dir: &Path,
		main_src: &str,
		cache: ModuleCache,
	) -> (Vec<String>, std::collections::HashSet<String>, ModuleCache) {
		let main_path = dir.join("main.pa");
		let mut compiler = match Compiler::from_entry_path(main_path.to_str().unwrap().to_string()) {
			Ok(c) => c,
			Err(_) => panic!("compiler init failed"),
		};
		let entry = compiler.entry_modules.first().cloned().unwrap();
		compiler.set_module_source(entry, main_src.as_bytes().to_vec());
		STDLIB_EXPORTS.with(|exports| compiler.seed_exports(exports));
		compiler.enable_incremental(cache);
		let diags = compiler.check().err().unwrap_or_default();
		let reanalyzed = compiler.reanalyzed_modules().clone();
		let cache = compiler.take_incremental_cache();
		(render(&diags), reanalyzed, cache)
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

	// A three-module chain main → mid → leaf, exercised across edits, pins the
	// whole incremental contract:
	//   * editing only the entry reuses both dependencies (neither reanalyzed);
	//   * changing a leaf's signature reanalyzes it AND every dependent, even
	//     though their own source is untouched (transitive invalidation);
	//   * the diagnostics from every incremental pass match a cold compile of
	//     the same sources (observational transparency).
	#[test]
	fn incremental_reuse_invalidation_and_transparency() {
		let dir = unique_dir("incr");
		let main_v1 = "use mid\n\ndef main = fun {\n\t(mid.via-leaf ()) + 1\n}\n";
		let mid = "use leaf\n\npublic def via-leaf = fun {\n\tleaf.base ()\n}\n";
		let leaf_int = "public def base = fun {\n\t10\n}\n";
		// Same shape, but `base` now yields a string — so `mid.via-leaf` is a
		// string and `... + 1` in `main` becomes a type error.
		let leaf_str = "public def base = fun {\n\t\"hello\"\n}\n";

		std::fs::write(dir.join("main.pa"), main_v1).unwrap();
		std::fs::write(dir.join("mid.pa"), mid).unwrap();
		std::fs::write(dir.join("leaf.pa"), leaf_int).unwrap();

		// Pass 1 (cold): the whole chain is analyzed, no diagnostics.
		let (d1, r1, cache) = incr_pass(&dir, main_v1, ModuleCache::new());
		assert!(d1.is_empty(), "pass 1 should be clean, got {d1:?}");
		assert!(r1.contains("main") && r1.contains("mid") && r1.contains("leaf"));

		// Pass 2: edit only the entry (a trailing comment). Both dependencies
		// are unchanged, so neither is reanalyzed — only `main`.
		let main_v2 = "use mid\n\ndef main = fun {\n\t(mid.via-leaf ()) + 1 # tweak\n}\n";
		let (d2, r2, cache) = incr_pass(&dir, main_v2, cache);
		assert!(d2.is_empty(), "pass 2 should be clean, got {d2:?}");
		assert!(r2.contains("main"), "entry is always reanalyzed");
		assert!(
			!r2.contains("mid") && !r2.contains("leaf"),
			"unchanged dependencies must be reused, but reanalyzed = {r2:?}"
		);

		// Pass 3: change `leaf` on disk. It must be reanalyzed, and so must
		// `mid` (its dependent) and `main` — and the new error must surface.
		std::fs::write(dir.join("leaf.pa"), leaf_str).unwrap();
		let (d3, r3, _) = incr_pass(&dir, main_v2, cache);
		assert!(
			r3.contains("leaf") && r3.contains("mid") && r3.contains("main"),
			"a signature change must reanalyze the changed module and all \
			 dependents, but reanalyzed = {r3:?}"
		);
		assert!(
			!d3.is_empty(),
			"pass 3 should report the int/string mismatch surfaced through mid"
		);

		// Transparency: the incremental result equals a cold compile of the
		// final on-disk state (incremental cache never enabled).
		let (cold, _, _) = incr_pass(&dir, main_v2, ModuleCache::new());
		assert_eq!(
			d3, cold,
			"incremental diagnostics diverged from a cold compile"
		);

		std::fs::remove_dir_all(&dir).ok();
	}

	// Not a correctness assert — shows the editing-loop cost when a file with
	// several user-module imports is edited repeatedly, with vs without the
	// incremental cache. Ignored by default (machine-dependent).
	#[test]
	#[ignore]
	fn incremental_speedup_timing() {
		let dir = unique_dir("incr-timing");
		// A handful of user modules, each non-trivial enough to cost real
		// analysis time, all imported by the entry.
		let mods = ["a", "b", "c", "d", "e"];
		let mut uses = String::new();
		let mut body = String::from("\t0");
		for m in mods {
			std::fs::write(
				dir.join(format!("{m}.pa")),
				format!("public def f-{m} = fun x {{\n\tlet y = x + 1\n\tlet z = y * 2\n\tz - x\n}}\n"),
			)
			.unwrap();
			uses.push_str(&format!("use {m}\n"));
			body = format!("\t({}.f-{} 1) + {}", m, m, body.trim());
		}
		let main = format!("{uses}\ndef main = fun {{\n{body}\n}}\n");
		std::fs::write(dir.join("main.pa"), &main).unwrap();

		STDLIB_EXPORTS.with(|_| {});
		let reps = 50;

		// Cold: fresh empty cache each pass → every dependency reanalyzed.
		let t = std::time::Instant::now();
		for i in 0..reps {
			let edited = format!("{main}# edit {i}\n");
			let _ = incr_pass(&dir, &edited, ModuleCache::new());
		}
		let cold = t.elapsed() / reps;

		// Incremental: carry the cache forward, editing only the entry.
		let mut cache = ModuleCache::new();
		let (_, _, c0) = incr_pass(&dir, &main, cache);
		cache = c0;
		let t = std::time::Instant::now();
		for i in 0..reps {
			let edited = format!("{main}# edit {i}\n");
			let (_, _, c) = incr_pass(&dir, &edited, cache);
			cache = c;
		}
		let incremental = t.elapsed() / reps;

		eprintln!("cold (reanalyze all user deps): {cold:?}/edit");
		eprintln!("incremental (reuse deps):       {incremental:?}/edit");

		std::fs::remove_dir_all(&dir).ok();
	}
}
