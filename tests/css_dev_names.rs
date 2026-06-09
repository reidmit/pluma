// `pluma dev` CSS class naming.
//
// Under `hmr` (the `pluma dev` rewrite mode), the analyzer wraps a top-level
// `css.ruleset` def's value in `css.label "<def-name>" (…)`, so the rule's generated
// class reads `card-<hash>` in browser devtools instead of a bare `c<hash>`. A normal
// build (no `hmr`) carries no label and keeps the bare hash — matching what `pluma
// build` ships. The label is content-neutral: it only prefixes the class, never the
// content hash, so the trailing hash (and the rendered CSS) is identical in both modes.
//
// The ungated run-suite compiles like `pluma run`, so it can't see this path; this
// test drives the compiler with `hmr` on and off explicitly and diffs the printed
// class names. In-memory (no temp files / cwd mutation) so it's parallel-safe.

use compiler::Compiler;

/// Compile `src` (with `hmr` on/off) to WasmGC, run it under V8, and return stdout.
fn run(src: &str, hmr: bool) -> String {
	let mut compiler = Compiler::for_root_dir(std::env::temp_dir()).with_hmr(hmr);
	compiler.add_entry_module("main".into());
	compiler.set_module_source("main".into(), src.as_bytes().to_vec());
	compiler.check().unwrap_or_else(|d| {
		panic!(
			"type-check failed (hmr={hmr}): {}",
			d.iter()
				.map(|x| x.message.clone())
				.collect::<Vec<_>>()
				.join("; ")
		)
	});
	let ir = ir::lower(&compiler).unwrap_or_else(|m| panic!("lower failed (hmr={hmr}): {m}"));
	let bytes = wasm::emit_with_options(&ir, wasm::EmitOptions::default())
		.unwrap_or_else(|d| panic!("emit failed (hmr={hmr}): {}", d.0.join("; ")));
	host::run_wasm_v8_captured(&bytes, &[]).stdout
}

// A `using css { ... }` block, a bare `css.rule`, and a `css.compose` — the three
// shapes the wrapper recognizes — each printing its `class-of`.
const SRC: &str = r##"
use std.css

def card :: css.ruleset = using css {
	.rule [
		.padding (.rem 1.5),
		.color (.hex "#fff"),
	]
}

def plain :: css.ruleset = css.rule [
	css.color (css.hex "#fff"),
]

def combo :: css.ruleset = css.compose [card, plain]

def main = fun {
	print (css.class-of card)
	print (css.class-of plain)
	print (css.class-of combo)
}
"##;

#[test]
fn release_build_keeps_bare_hashes() {
	let out = run(SRC, false);
	let lines: Vec<&str> = out.lines().collect();
	assert_eq!(lines.len(), 3, "expected three class names, got: {out:?}");
	for line in &lines {
		assert!(
			line.starts_with('c') && !line.contains('-'),
			"release class should be a bare `c<hash>`, got {line:?}"
		);
	}
}

#[test]
fn dev_build_prefixes_the_def_name() {
	let out = run(SRC, true);
	let lines: Vec<&str> = out.lines().collect();
	assert_eq!(lines.len(), 3, "expected three class names, got: {out:?}");
	assert!(lines[0].starts_with("card-"), "got {:?}", lines[0]);
	assert!(lines[1].starts_with("plain-"), "got {:?}", lines[1]);
	assert!(lines[2].starts_with("combo-"), "got {:?}", lines[2]);
}

// The label is content-neutral: the trailing hash a dev class carries is exactly the
// release class with its leading `c` dropped (both are `c<hash>` content hashes), so
// dedup and the emitted CSS are unaffected by the debug name.
#[test]
fn dev_name_does_not_change_the_content_hash() {
	let release = run(SRC, false);
	let dev = run(SRC, true);
	for (rel, dv) in release.lines().zip(dev.lines()) {
		// release: `c<hash>`  →  hash = rel without the leading `c`.
		// dev:     `<name>-<hash>`  →  hash = the segment after the last `-`.
		let rel_hash = rel.strip_prefix('c').expect("release class starts with c");
		let dev_hash = dv.rsplit('-').next().expect("dev class has a hash");
		assert_eq!(
			rel_hash, dev_hash,
			"hash diverged: release {rel:?} vs dev {dv:?}"
		);
	}
}
