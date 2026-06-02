// Perf measurement across the VM and WasmGC/V8 backends. Two views:
//   - dev_loop: the `tests/run` corpus minus the `bench`-marked fixtures (tiny
//     programs) — the cost of one `pluma test`-style pass per backend, with a
//     cold/warm split for wasm.
//   - compute: the `bench`-marked `tests/run` fixtures (longer programs) —
//     steady-state throughput per backend.
//
// The wasm numbers are measured under V8 (the deploy engine): `wasm_exec` is warm
// (module compiled once, fresh instance per run, via `host::bench_exec_v8`); `wasm_e2e`
// is cold (a fresh isolate + V8 module-compile + run per invocation, via
// `host::run_wasm_v8`) — the `pluma run` / cold-start cost.
//
// Run with --release; a debug build runs V8 unoptimized and is far slower.

use std::time::{Duration, Instant};

use compiler::Platform;

use crate::{perf_corpus, run, run_corpus};

fn iters() -> u32 {
	std::env::var("BENCH_ITERS")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(3)
}

pub fn fmt_dur(d: Duration) -> String {
	let us = d.as_secs_f64() * 1_000_000.0;
	if us < 1000.0 {
		format!("{us:.1}us")
	} else if us < 1_000_000.0 {
		format!("{:.2}ms", us / 1000.0)
	} else {
		format!("{:.3}s", us / 1_000_000.0)
	}
}

// ---- dev-loop (whole corpus, tiny programs) ------------------------------

pub struct DevLoop {
	pub fixtures: u32,
	pub iters: u32,
	pub frontend: Duration,
	pub vm: Duration,
	pub wasm_e2e: Duration,
	pub wasm_exec: Duration,
	pub wasm_n: u32,
}

pub fn dev_loop() -> DevLoop {
	let it = iters();
	let mut d = DevLoop {
		fixtures: 0,
		iters: it,
		frontend: Duration::ZERO,
		vm: Duration::ZERO,
		wasm_e2e: Duration::ZERO,
		wasm_exec: Duration::ZERO,
		wasm_n: 0,
	};

	for dir in run_corpus() {
		let name = dir.file_name().unwrap().to_string_lossy().into_owned();
		// `io-*` touch the filesystem — skip in a hot loop (noisy + side-effecting).
		if name.starts_with("io-") {
			continue;
		}
		// `bench`-marked fixtures are the longer compute programs — they belong to
		// the `compute` view, not this tiny-program dev-loop measurement.
		if crate::is_bench(&dir) {
			continue;
		}
		let f_start = Instant::now();
		let ir = match run::compile(&dir, Platform::Native) {
			Ok(ir) => ir,
			Err(_) => continue,
		};
		d.frontend += f_start.elapsed();
		d.fixtures += 1;
		let stdin = std::fs::read(dir.join("stdin.txt")).unwrap_or_default();

		// VM end-to-end: bytecode codegen + run.
		for _ in 0..it {
			let s = Instant::now();
			if let Ok(p) = codegen::compile_from_ir(&ir) {
				let _ = run::run_vm(p, &stdin);
			}
			d.vm += s.elapsed();
		}

		// WASM under V8: warm exec (module compiled once, fresh instance per run) and
		// cold e2e (emit + fresh-isolate V8 compile + run — the `pluma run` cost).
		if let Ok(bytes) = wasm::emit(&ir) {
			if let Some(durs) = host::bench_exec_v8(&bytes, &stdin, it) {
				d.wasm_n += 1;
				d.wasm_exec += durs.iter().sum::<Duration>();
				for _ in 0..it {
					let s = Instant::now();
					let b = wasm::emit(&ir).unwrap();
					let _ = host::run_wasm_v8(&b, &stdin);
					d.wasm_e2e += s.elapsed();
				}
			}
		}
	}

	// Each accumulator summed `iters` runs per fixture; normalize to one pass.
	d.vm /= it;
	d.wasm_e2e /= it;
	d.wasm_exec /= it;
	d
}

// ---- compute (perf corpus, longer programs) ------------------------------

pub struct ComputeRow {
	pub name: String,
	pub vm: Option<Duration>,
	pub wasm_exec: Option<Duration>,
	pub wasm_e2e: Option<Duration>,
}

pub fn compute() -> Vec<ComputeRow> {
	let it = iters().max(5);
	let mut rows = Vec::new();
	for dir in perf_corpus() {
		let name = dir.file_name().unwrap().to_string_lossy().into_owned();
		let Ok(ir) = run::compile(&dir, Platform::Native) else {
			continue;
		};
		let stdin = std::fs::read(dir.join("stdin.txt")).unwrap_or_default();
		let mut row = ComputeRow {
			name,
			vm: None,
			wasm_exec: None,
			wasm_e2e: None,
		};

		// VM e2e
		let mut t = Duration::ZERO;
		for _ in 0..it {
			let s = Instant::now();
			if let Ok(p) = codegen::compile_from_ir(&ir) {
				let _ = run::run_vm(p, &stdin);
			}
			t += s.elapsed();
		}
		row.vm = Some(t / it);

		// WASM under V8: warm exec + cold e2e.
		if let Ok(bytes) = wasm::emit(&ir) {
			if let Some(durs) = host::bench_exec_v8(&bytes, &stdin, it) {
				row.wasm_exec = Some(durs.iter().sum::<Duration>() / it);
				let mut t2 = Duration::ZERO;
				for _ in 0..it {
					let s = Instant::now();
					let b = wasm::emit(&ir).unwrap();
					let _ = host::run_wasm_v8(&b, &stdin);
					t2 += s.elapsed();
				}
				row.wasm_e2e = Some(t2 / it);
			}
		}

		rows.push(row);
	}
	rows
}
