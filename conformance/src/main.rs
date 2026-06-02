// The conformance report CLI.
//
//   cargo run -p conformance --release            # correctness + perf; regen CONFORMANCE.md + target report
//   cargo run -p conformance -- --check           # correctness only; verify CONFORMANCE.md is fresh
//   cargo run -p conformance --release -- --perf   # perf only
//
// Correctness is also a `cargo test -p conformance` gate (tests/correctness.rs).

use std::fmt::Write as _;

use conformance::report::{self, Coverage};
use conformance::{Runner, check_fixture, correctness_corpus, perf, workspace_root};

fn main() {
	// Modes: default = correctness + (re)write CONFORMANCE.md (fast, no perf);
	// --check = correctness + verify CONFORMANCE.md is fresh (CI, no write);
	// --perf = the above plus the perf tables → target/conformance/report.md.
	let args: Vec<String> = std::env::args().skip(1).collect();
	let check_only = args.iter().any(|a| a == "--check");
	let do_perf = args.iter().any(|a| a == "--perf");

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

	if do_perf {
		eprintln!("running perf (this is slow; --release strongly recommended)...");
		let dev = perf::dev_loop();
		let compute = perf::compute();
		let report_md = render_full_report(Some(&cov), &dev, &compute);
		let dir = workspace_root().join("target/conformance");
		std::fs::create_dir_all(&dir).ok();
		let report_path = dir.join("report.md");
		std::fs::write(&report_path, &report_md).expect("write report.md");
		eprintln!("wrote {}", report_path.display());
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

/// The full, on-demand report: the committed coverage matrix plus both perf
/// tables and the environment. Written to target/ (not committed).
fn render_full_report(
	cov: Option<&Coverage>,
	dev: &perf::DevLoop,
	compute: &[perf::ComputeRow],
) -> String {
	use perf::fmt_dur;
	let mut s = String::new();
	s.push_str("# Pluma conformance + perf report\n\n");
	s.push_str(&format!(
		"wasm engine: V8 (the deploy engine) · perf iters/fixture: {} (compute: ≥5)\n\n",
		dev.iters,
	));

	if let Some(cov) = cov {
		s.push_str("## Correctness (vs VM oracle)\n\n");
		s.push_str("| Backend | Match | Diverge | Skipped |\n|---|---:|---:|---:|\n");
		for c in &cov.backends {
			let skips: usize = c.skips.values().map(|v| v.len()).sum();
			let _ = writeln!(
				s,
				"| {} | {} | {} | {} |",
				c.backend.name(),
				c.matched,
				c.diverged.len(),
				skips
			);
		}
		s.push('\n');
	}

	// Dev-loop perf.
	s.push_str("## Dev-loop cost — whole tests/run corpus (one pass)\n\n");
	let _ = writeln!(
		s,
		"{} fixtures (io-* excluded). The VM is the dev/test engine; the wasm e2e number\npays a fresh V8 isolate + module compile per program (the `pluma run` cost), while\nexec is warm (module compiled once).\n",
		dev.fixtures
	);
	s.push_str("| Stage | Total | per fixture |\n|---|---:|---:|\n");
	let per = |d: std::time::Duration, n: u32| {
		if n == 0 {
			"—".to_string()
		} else {
			fmt_dur(d / n)
		}
	};
	let _ = writeln!(
		s,
		"| frontend (shared) | {} | {} |",
		fmt_dur(dev.frontend),
		per(dev.frontend, dev.fixtures)
	);
	let _ = writeln!(
		s,
		"| VM e2e (codegen+run) | {} | {} |",
		fmt_dur(dev.vm),
		per(dev.vm, dev.fixtures)
	);
	let _ = writeln!(
		s,
		"| WASM e2e (emit+V8 compile+run, cold) | {} | {} |",
		fmt_dur(dev.wasm_e2e),
		per(dev.wasm_e2e, dev.wasm_n)
	);
	let _ = writeln!(
		s,
		"| WASM exec (warm, module cached) | {} | {} |",
		fmt_dur(dev.wasm_exec),
		per(dev.wasm_exec, dev.wasm_n)
	);
	s.push('\n');
	if dev.vm.as_secs_f64() > 0.0 {
		let _ = writeln!(
			s,
			"- WASM e2e / VM e2e = **{:.1}x**, WASM exec / VM e2e = **{:.2}x**",
			dev.wasm_e2e.as_secs_f64() / dev.vm.as_secs_f64(),
			dev.wasm_exec.as_secs_f64() / dev.vm.as_secs_f64(),
		);
	}
	s.push('\n');

	// Compute perf.
	s.push_str("## Compute throughput — bench-marked tests/run fixtures\n\n");
	s.push_str("| Program | VM | WASM exec | WASM e2e |\n|---|---:|---:|---:|\n");
	let cell = |d: Option<std::time::Duration>| d.map(fmt_dur).unwrap_or_else(|| "n/a".into());
	for r in compute {
		let _ = writeln!(
			s,
			"| {} | {} | {} | {} |",
			r.name,
			cell(r.vm),
			cell(r.wasm_exec),
			cell(r.wasm_e2e),
		);
	}
	s.push_str("\n_WASM exec is the warm deploy number (module compiled once, reused per request); e2e is the cold-start cost (fresh isolate + compile + run)._\n");
	s
}
