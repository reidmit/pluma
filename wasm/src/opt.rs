//! Optional post-emit pass over the encoded module through Binaryen's `wasm-opt`.
//!
//! The emitter produces correct-but-naive WasmGC; `wasm-opt` runs Binaryen's
//! whole-module optimization pipeline (inlining, DCE, local coalescing, GC-aware
//! cleanups) over the finished bytes. It's opt-in (`pluma build -O`) because it
//! pulls in a heavy native dependency and adds noticeable build latency — the
//! interesting question is how much the runtime artifact gains for that cost.

use wasm_opt::OptimizationOptions;

/// How hard `wasm-opt` works. Maps onto Binaryen's `-O2/-O3/-O4` (speed) and
/// `-Os/-Oz` (size) presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptLevel {
	O2,
	O3,
	O4,
	Os,
	Oz,
}

impl OptLevel {
	/// Parse the `-O <LEVEL>` argument; `None` for an unrecognized token.
	pub fn parse(s: &str) -> Option<OptLevel> {
		match s {
			"2" => Some(OptLevel::O2),
			"3" => Some(OptLevel::O3),
			"4" => Some(OptLevel::O4),
			"s" | "S" => Some(OptLevel::Os),
			"z" | "Z" => Some(OptLevel::Oz),
			_ => None,
		}
	}
}

/// Optimize `bytes` with `wasm-opt` at `level`, returning the rewritten module.
///
/// Binaryen's Rust binding only reads/writes files, so we round-trip through a
/// uniquely-named pair in the temp dir. Every WasmGC feature is enabled
/// (`all_features`) — our module uses GC structs/arrays, tail calls, and
/// reference types, all of which Binaryen rejects unless explicitly turned on.
pub fn optimize(bytes: &[u8], level: OptLevel) -> Result<Vec<u8>, String> {
	use std::sync::atomic::{AtomicU64, Ordering};
	// pid + a process-local counter keeps concurrent emits (the test harness) from
	// colliding on the temp paths without needing a random source.
	static SEQ: AtomicU64 = AtomicU64::new(0);
	let n = SEQ.fetch_add(1, Ordering::Relaxed);
	let pid = std::process::id();
	let dir = std::env::temp_dir();
	let infile = dir.join(format!("pluma-opt-{pid}-{n}.in.wasm"));
	let outfile = dir.join(format!("pluma-opt-{pid}-{n}.out.wasm"));

	std::fs::write(&infile, bytes).map_err(|e| format!("wasm-opt: writing input: {e}"))?;

	let mut opts = match level {
		OptLevel::O2 => OptimizationOptions::new_opt_level_2(),
		OptLevel::O3 => OptimizationOptions::new_opt_level_3(),
		OptLevel::O4 => OptimizationOptions::new_opt_level_4(),
		OptLevel::Os => OptimizationOptions::new_optimize_for_size(),
		OptLevel::Oz => OptimizationOptions::new_optimize_for_size_aggressively(),
	};
	opts.all_features();

	let run = opts.run(&infile, &outfile);
	let _ = std::fs::remove_file(&infile);
	run.map_err(|e| format!("wasm-opt: {e}"))?;

	let out = std::fs::read(&outfile).map_err(|e| format!("wasm-opt: reading output: {e}"))?;
	let _ = std::fs::remove_file(&outfile);
	Ok(out)
}
