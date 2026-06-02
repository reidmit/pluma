// The cross-backend conformance harness. Runs each Pluma program through the VM
// (oracle) and the WasmGC deploy backend (under V8); the deploy backend is diffed
// against the VM and the result rendered into `CONFORMANCE.md` (see `report`). This is
// the standing correctness reference as features land across the two runtimes. (Perf is
// measured elsewhere: `competition/` for process-level deploy timing, `bench/` for VM
// compute throughput.)

mod run;

pub mod report;

use std::path::{Path, PathBuf};

use compiler::Platform;
/// The WasmGC runtime lives in the shared `host` crate (the same one `cli` ships):
/// the differential gate runs every fixture through exactly the runtime the CLI
/// uses. `RunResult` is its result type, re-exported here as the unit of comparison.
pub use host::RunResult;

/// The execution backends, both consuming the same `ir::IrProgram`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
	/// The bytecode VM — the reference/oracle and dev/test engine.
	Vm,
	/// The WasmGC deploy artifact, run under V8 (the deploy engine — server/compute,
	/// and the browser/client target as the frontend lands). Diffed against the VM to
	/// pin the marshalling ABI + codegen on every fixture.
	Wasm,
}

impl Backend {
	/// The deploy backends, diffed against the VM oracle.
	pub const DEPLOY: [Backend; 1] = [Backend::Wasm];

	pub fn name(self) -> &'static str {
		match self {
			Backend::Vm => "VM",
			Backend::Wasm => "WasmGC",
		}
	}

	/// The host-capability profile each backend targets (gates which stdlib
	/// modules a fixture may `use`).
	pub fn platform(self) -> Platform {
		match self {
			Backend::Vm => Platform::Native,
			Backend::Wasm => Platform::Server,
		}
	}
}

/// What happened when a backend was asked to run a fixture.
pub enum Outcome {
	/// Ran to completion with this result.
	Ran(RunResult),
	/// Did not run, for a recorded reason.
	Skip(SkipReason),
}

/// Why a backend skipped a fixture. Each is a tracked, reported category — never
/// a silent gap.
#[derive(Clone)]
pub enum SkipReason {
	/// A `use`d module needs host capabilities this backend's platform lacks
	/// (e.g. `core.io` on the browser). Carries the diagnostic.
	Gated(String),
	/// The backend can't yet lower this program (e.g. async `Await` on JS, or an
	/// unsupported builtin on wasm). Carries why.
	Unsupported(String),
	/// A known, documented divergence from the VM the backend doesn't yet match.
	/// Carries why. Shrinking this set is the to-do list.
	Denied(String),
	/// The fixture has no VM reference (a compile-error fixture, owned by the
	/// run-snapshot suite, not this cross-backend one).
	NoReference,
}

impl SkipReason {
	/// A short category label for grouping in the report.
	pub fn category(&self) -> &'static str {
		match self {
			SkipReason::Gated(_) => "gated",
			SkipReason::Unsupported(_) => "unsupported",
			SkipReason::Denied(_) => "denied",
			SkipReason::NoReference => "no-reference",
		}
	}

	pub fn detail(&self) -> &str {
		match self {
			SkipReason::Gated(s) | SkipReason::Unsupported(s) | SkipReason::Denied(s) => s,
			SkipReason::NoReference => "compile-error fixture",
		}
	}
}

/// Known output divergences a deploy backend doesn't yet match the VM on (the
/// `denied` skip category). Currently empty — every deploy backend that lowers a
/// fixture matches the VM. Genuine "can't lower it" gaps surface dynamically as
/// `Unsupported` (`wasm::emit` returning `Err`), not here.
fn denied(backend: Backend, name: &str) -> Option<&'static str> {
	let _ = (backend, name);
	None
}

/// Drives the backends. Pins cwd at the workspace root so `core.io` fixtures
/// resolve relative paths identically across backends.
pub struct Runner {}

impl Default for Runner {
	fn default() -> Self {
		Self::new()
	}
}

impl Runner {
	pub fn new() -> Self {
		let _ = std::env::set_current_dir(workspace_root());
		Runner {}
	}

	/// Run one fixture through one backend.
	pub fn run(&self, backend: Backend, dir: &Path) -> Outcome {
		let name = fixture_name(dir);
		let ir = match run::compile(dir, backend.platform()) {
			Ok(ir) => ir,
			Err(msgs) => {
				// Compiles on Native but not here ⇒ gated; fails on Native too ⇒ a
				// compile-error fixture (no reference).
				return if run::compile(dir, Platform::Native).is_err() {
					Outcome::Skip(SkipReason::NoReference)
				} else {
					Outcome::Skip(SkipReason::Gated(first_line(&msgs)))
				};
			}
		};
		if let Some(why) = denied(backend, &name) {
			return Outcome::Skip(SkipReason::Denied(why.to_string()));
		}
		let stdin = std::fs::read(dir.join("stdin.txt")).unwrap_or_default();
		match backend {
			// The VM oracle applies its VM-specific pipeline (`ir::optimize`: inline +
			// direct calls + M6 monomorphization/unboxing) on top of the raw IR, just
			// as `pluma run` does. The deploy backend below runs its own internal
			// pipeline straight off the raw `ir` instead.
			Backend::Vm => {
				let mut ir = ir;
				ir::optimize(&mut ir);
				match codegen::compile_from_ir(&ir) {
					Ok(p) => Outcome::Ran(run::run_vm(p, &stdin)),
					Err(e) => Outcome::Skip(SkipReason::Unsupported(e.to_string())),
				}
			}
			// Emit the WasmGC artifact and run it under V8 — exactly what `pluma run`
			// ships and executes.
			Backend::Wasm => match wasm::emit(&ir) {
				Ok(bytes) => Outcome::Ran(host::run_wasm_v8(&bytes, &stdin)),
				Err(d) => Outcome::Skip(SkipReason::Unsupported(format!(
					"wasm::emit rejected ({} diag)",
					d.0.len()
				))),
			},
		}
	}
}

/// The cross-backend result for one fixture: the VM oracle plus each deploy
/// backend's outcome and whether it matched.
pub struct FixtureResult {
	pub name: String,
	/// `None` ⇒ the fixture has no VM reference (compile-error fixture; excluded).
	pub oracle: Option<RunResult>,
	pub backends: Vec<BackendResult>,
}

pub struct BackendResult {
	pub backend: Backend,
	pub outcome: Outcome,
	/// `Some(true)` matched the oracle, `Some(false)` diverged, `None` skipped.
	pub matched: Option<bool>,
}

/// Run a fixture through the VM oracle and each deploy backend, recording match
/// or skip.
pub fn check_fixture(runner: &Runner, dir: &Path) -> FixtureResult {
	let name = fixture_name(dir);
	let oracle = match runner.run(Backend::Vm, dir) {
		Outcome::Ran(r) => r,
		Outcome::Skip(_) => {
			return FixtureResult {
				name,
				oracle: None,
				backends: Vec::new(),
			};
		}
	};
	let backends = Backend::DEPLOY
		.iter()
		.map(|&b| {
			let outcome = runner.run(b, dir);
			let matched = match &outcome {
				Outcome::Ran(r) => Some(r == &oracle),
				Outcome::Skip(_) => None,
			};
			BackendResult {
				backend: b,
				outcome,
				matched,
			}
		})
		.collect();
	FixtureResult {
		name,
		oracle: Some(oracle),
		backends,
	}
}

// ---- corpus --------------------------------------------------------------

/// The happy-path execution corpus: every `tests/run/<name>/main.pa`, sorted.
/// Status `ok` programs only — also the perf dev-loop's corpus (perf benches
/// successful runs, not failures).
pub fn run_corpus() -> Vec<PathBuf> {
	fixtures_under(&workspace_root().join("tests/run"))
}

/// The runtime-failure corpus: every `tests/run-fail/<name>/main.pa`, sorted.
/// Programs that compile but fail at runtime — diffed across backends for error
/// parity, but excluded from perf.
pub fn fail_corpus() -> Vec<PathBuf> {
	fixtures_under(&workspace_root().join("tests/run-fail"))
}

/// The cross-backend correctness corpus: every execution fixture (`tests/run` +
/// `tests/run-fail`), sorted. The deploy backends are diffed against the VM
/// oracle over this whole set.
pub fn correctness_corpus() -> Vec<PathBuf> {
	let mut dirs = run_corpus();
	dirs.extend(fail_corpus());
	dirs.sort();
	dirs
}

/// The compute-perf corpus: the `tests/run` fixtures that carry a `bench` marker
/// file — the longer, steady-state-throughput stress programs (folded into
/// `tests/run` so they also get snapshot + cross-backend correctness coverage).
/// The dev-loop view excludes these so it stays a tiny-program measurement.
pub fn perf_corpus() -> Vec<PathBuf> {
	run_corpus().into_iter().filter(|d| is_bench(d)).collect()
}

/// Whether a fixture dir is a compute benchmark (carries the `bench` marker).
pub fn is_bench(dir: &Path) -> bool {
	dir.join("bench").exists()
}

fn fixtures_under(root: &Path) -> Vec<PathBuf> {
	let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
		.unwrap_or_else(|_| panic!("corpus dir not found: {}", root.display()))
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.join("main.pa").exists())
		.collect();
	dirs.sort();
	dirs
}

pub fn workspace_root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn fixture_name(dir: &Path) -> String {
	dir.file_name().unwrap().to_string_lossy().into_owned()
}

fn first_line(msgs: &[String]) -> String {
	msgs
		.first()
		.cloned()
		.unwrap_or_else(|| "compile error".into())
}
