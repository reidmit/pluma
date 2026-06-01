// The cross-backend correctness gate (replaces the old wasm_diff/js_diff
// `#[test]`s). Runs every `tests/run` fixture through the VM oracle, WasmGC, and
// JS, and asserts the deploy backends match the VM. Also asserts the committed
// CONFORMANCE.md is fresh (when node is present, so the JS rows are real).
//
// Wrapped in a roomy-stack thread: the VM nests a Rust frame per Pluma call (no
// TCO), so the `deep-recursion` fixture would overflow the default 2 MiB test
// thread otherwise.

use conformance::{Runner, check_fixture, correctness_corpus, report, workspace_root};

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
	if !runner.has_node() {
		eprintln!("note: `node` absent — JS rows skipped (set $PLUMA_NODE to include them).");
	}

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

	let diffs = report::divergences(&cov);
	assert!(
		diffs.is_empty(),
		"a deploy backend diverged from the VM oracle:\n{}",
		diffs.join("\n")
	);

	// The committed coverage doc must match a fresh render (snapshot-style). Only
	// when node is present — otherwise the JS rows would be unmeasured and the
	// render non-deterministic.
	if runner.has_node() {
		let fresh = report::render_conformance_md(&cov);
		let path = workspace_root().join("CONFORMANCE.md");
		let current = std::fs::read_to_string(&path).unwrap_or_default();
		assert_eq!(
			current, fresh,
			"CONFORMANCE.md is stale — run `just conformance` and commit the result."
		);
	}
}
