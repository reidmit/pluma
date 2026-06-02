// The cross-backend correctness gate. Runs every `tests/run` fixture through the
// VM oracle and the WasmGC deploy backend, and asserts WasmGC matches the VM.
// Also asserts the committed CONFORMANCE.md is fresh.
//
// Wrapped in a roomy-stack thread: the VM nests a Rust frame per Pluma call (no
// TCO), so the `deep-recursion` fixture would overflow the default 2 MiB test
// thread otherwise.

use conformance::{Runner, check_fixture, correctness_corpus, report};
// Only the default (non-v8) gate re-renders + freshness-checks CONFORMANCE.md.
#[cfg(not(feature = "v8"))]
use conformance::workspace_root;

#[test]
fn deploy_backends_match_vm_oracle() {
	std::thread::Builder::new()
		.stack_size(256 * 1024 * 1024)
		.spawn(body)
		.unwrap()
		.join()
		.unwrap();
}

fn body() {
	let runner = Runner::new();

	let results: Vec<_> = correctness_corpus()
		.iter()
		.map(|d| check_fixture(&runner, d))
		.collect();
	let cov = report::coverage(&results);

	eprintln!(
		"conformance ({} execution fixtures, VM = oracle):",
		cov.run_fixtures
	);
	for c in &cov.backends {
		let skips: usize = c.skips.values().map(|v| v.len()).sum();
		eprintln!(
			"  {:<8} {} match / {} diverge / {} skip",
			c.backend.name(),
			c.matched,
			c.diverged.len(),
			skips
		);
	}

	// Every deploy backend in the corpus must match the VM — wasmtime always, and V8
	// too under `--features v8` (the V8↔VM cross-check over the same artifact).
	let diffs = report::divergences(&cov);
	assert!(
		diffs.is_empty(),
		"a deploy backend diverged from the VM oracle:\n{}",
		diffs.join("\n")
	);

	// The committed coverage doc tracks the default (wasmtime) backend set; only check
	// its freshness there (with `--features v8` the render would gain a V8 row).
	#[cfg(not(feature = "v8"))]
	{
		let fresh = report::render_conformance_md(&cov);
		let path = workspace_root().join("CONFORMANCE.md");
		let current = std::fs::read_to_string(&path).unwrap_or_default();
		assert_eq!(
			current, fresh,
			"CONFORMANCE.md is stale — run `just conformance` and commit the result."
		);
	}
}
