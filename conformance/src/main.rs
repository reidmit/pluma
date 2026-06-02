// The conformance report CLI.
//
//   cargo run -p conformance --release     # correctness; (re)write CONFORMANCE.md
//   cargo run -p conformance -- --check    # correctness only; verify CONFORMANCE.md is fresh
//
// Correctness is also a `cargo test -p conformance` gate (tests/correctness.rs).

use conformance::report::{self, Coverage};
use conformance::{Runner, check_fixture, correctness_corpus, workspace_root};

fn main() {
	// Modes: default = correctness + (re)write CONFORMANCE.md;
	// --check = correctness + verify CONFORMANCE.md is fresh (CI, no write).
	let args: Vec<String> = std::env::args().skip(1).collect();
	let check_only = args.iter().any(|a| a == "--check");

	let runner = Runner::new();
	let mut exit = 0;

	eprintln!("running correctness over the tests/run corpus...");
	let results: Vec<_> = correctness_corpus()
		.iter()
		.map(|d| check_fixture(&runner, d))
		.collect();
	let cov = report::coverage(&results);
	print_coverage_summary(&cov);

	let diffs = report::divergences(&cov);
	if !diffs.is_empty() {
		eprintln!("\nDIVERGENCES FROM THE VM ORACLE:\n{}", diffs.join("\n"));
		exit = 1;
	}

	// The committed coverage doc: write it (default) or verify it's fresh (--check).
	let md = report::render_conformance_md(&cov);
	let path = workspace_root().join("CONFORMANCE.md");
	if check_only {
		let current = std::fs::read_to_string(&path).unwrap_or_default();
		if current != md {
			eprintln!("\nCONFORMANCE.md is stale — run `just conformance` to regenerate.");
			exit = 1;
		}
	} else {
		std::fs::write(&path, &md).expect("write CONFORMANCE.md");
		eprintln!("wrote {}", path.display());
	}

	std::process::exit(exit);
}

fn print_coverage_summary(cov: &Coverage) {
	eprintln!(
		"\nCorrectness ({} execution fixtures, VM = oracle):",
		cov.run_fixtures
	);
	for c in &cov.backends {
		let skips: usize = c.skips.values().map(|v| v.len()).sum();
		eprintln!(
			"  {:<8} {:>3} match / {} diverge / {} skip",
			c.backend.name(),
			c.matched,
			c.diverged.len(),
			skips,
		);
	}
}
