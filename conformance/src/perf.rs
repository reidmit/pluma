// Perf measurement, ported from the (removed) wasm_diff.rs benches and extended
// to all three backends. Two views:
//   - dev_loop: the `tests/run` corpus minus the `bench`-marked fixtures (tiny
//     programs) — the cost of one `pluma test`-style pass per backend, incl. the
//     compile/exec split.
//   - compute: the `bench`-marked `tests/run` fixtures (longer programs) —
//     steady-state throughput per backend.
//
// JS is timed via the node subprocess (there is no in-process JS engine), so JS
// numbers include node process startup — labeled accordingly. Run with --release;
// debug cranelift is pathologically slow.

use std::time::{Duration, Instant};

use compiler::Platform;
use wasmtime::Module;

use crate::{Backend, Runner, js_host, perf_corpus, run, run_corpus};

fn iters() -> u32 {
	std::env::var("BENCH_ITERS")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(3)
}

/// The wasm engine for timing. `BENCH_WASM_GC=drc` switches to the deferred-
/// reference-counting collector (reclaims within a run) for allocation-heavy
/// programs that would otherwise trap on the fastest null collector.
fn perf_engine() -> wasmtime::Engine {
	if std::env::var("BENCH_WASM_GC").as_deref() == Ok("drc") {
		host::bench_engine()
	} else {
		host::engine()
	}
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
	pub js: Duration,
	pub js_n: u32,
}

pub fn dev_loop(runner: &Runner) -> DevLoop {
	let it = iters();
	let mut d = DevLoop {
		fixtures: 0,
		iters: it,
		frontend: Duration::ZERO,
		vm: Duration::ZERO,
		wasm_e2e: Duration::ZERO,
		wasm_exec: Duration::ZERO,
		wasm_n: 0,
		js: Duration::ZERO,
		js_n: 0,
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

		// WASM: emit + cranelift JIT + instantiate + run (e2e) and exec-only.
		if let Ok(bytes) = wasm::emit(&ir) {
			let engine = perf_engine();
			if let Ok(module) = Module::new(&engine, &bytes) {
				d.wasm_n += 1;
				for _ in 0..it {
					let s = Instant::now();
					let b = wasm::emit(&ir).unwrap();
					let m = Module::new(&engine, &b).unwrap();
					let _ = host::run_entry(&engine, &m, &stdin);
					d.wasm_e2e += s.elapsed();
				}
				for _ in 0..it {
					let s = Instant::now();
					let _ = host::run_entry(&engine, &module, &stdin);
					d.wasm_exec += s.elapsed();
				}
			}
		}

		// JS: node subprocess (includes node startup). Skip denied fixtures (they
		// may legitimately error under node, e.g. deep-recursion stack overflow).
		if let (Some(node), Ok(src)) = (runner.node.as_deref(), js::emit(&ir)) {
			if crate::denied(Backend::Js, &name).is_none() {
				d.js_n += 1;
				for _ in 0..it {
					let s = Instant::now();
					let _ = js_host::run_node(node, &src, &name);
					d.js += s.elapsed();
				}
			}
		}
	}

	// Each accumulator summed `iters` runs per fixture; normalize to one pass.
	d.vm /= it;
	d.wasm_e2e /= it;
	d.wasm_exec /= it;
	d.js /= it;
	d
}

// ---- compute (perf corpus, longer programs) ------------------------------

pub struct ComputeRow {
	pub name: String,
	pub vm: Option<Duration>,
	pub wasm_exec: Option<Duration>,
	pub wasm_e2e: Option<Duration>,
	pub js: Option<Duration>,
}

pub fn compute(runner: &Runner) -> Vec<ComputeRow> {
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
			js: None,
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

		// WASM exec + e2e
		if let Ok(bytes) = wasm::emit(&ir) {
			let engine = perf_engine();
			if let Ok(module) = Module::new(&engine, &bytes) {
				let mut te = Duration::ZERO;
				for _ in 0..it {
					let s = Instant::now();
					let _ = host::run_entry(&engine, &module, &stdin);
					te += s.elapsed();
				}
				row.wasm_exec = Some(te / it);
				let mut t2 = Duration::ZERO;
				for _ in 0..it {
					let s = Instant::now();
					let b = wasm::emit(&ir).unwrap();
					let m = Module::new(&engine, &b).unwrap();
					let _ = host::run_entry(&engine, &m, &stdin);
					t2 += s.elapsed();
				}
				row.wasm_e2e = Some(t2 / it);
			}
		}

		// JS (node e2e)
		if let (Some(node), Ok(src)) = (runner.node.as_deref(), js::emit(&ir)) {
			if crate::denied(Backend::Js, &row.name).is_none() {
				let mut t = Duration::ZERO;
				for _ in 0..it {
					let s = Instant::now();
					let _ = js_host::run_node(node, &src, &row.name);
					t += s.elapsed();
				}
				row.js = Some(t / it);
			}
		}

		rows.push(row);
	}
	rows
}
