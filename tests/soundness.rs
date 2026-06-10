// Reuse-soundness harness — the gate that earns trust in the in-place reuse pass
// (`ir::reuse`).
//
// The pass mutates a `dict.insert` accumulator in place when it can prove the dict is
// uniquely owned and dead-after. Getting that wrong is not a slow path — it is silent
// heap corruption: a value something else still holds gets mutated underneath it. The
// unit tests in `ir/src/reuse.rs` pin the analysis decisions; this harness pins the
// *observable consequence* end-to-end.
//
// The invariant: **reuse is observationally transparent.** For any program, compiling
// with reuse on must produce byte-identical output to compiling with it off (the
// persistent baseline — always a correct copy). So each case is compiled twice and run
// under V8 (the deploy engine), and the two outputs are diffed. This stays a valid
// invariant no matter how the analysis evolves: a future change that loosens it into
// firing on an aliased dict corrupts the in-place case, the persistent baseline stays
// correct, the outputs diverge, and this catches it.
//
// Two emits, not a process-global env toggle, so the parallel test harness can't race:
// `wasm::EmitOptions { reuse }` is a thread-safe value.
//
// The corpus deliberately mixes three regimes (and the meta-tests assert each is
// genuinely represented, so the suite can't silently go vacuous):
//   - FIRE, clean accumulator — exercises the transient `dict-insert-into` path.
//   - FIRE, aliased *input* — building on a pre-existing dict; the fresh edit token
//     makes the input's nodes foreign, so `__cnode_tinsert` must copy-on-write them.
//     This is where a freeze/COW bug would surface.
//   - DECLINE — the dict (or an intermediate) escapes, so the analysis must leave the
//     persistent copy. Currently transparent for free; the guard is that a regression
//     loosening the escape check would make the in-place mutation observable here.

use compiler::Compiler;

/// Whether a case is expected to fire the reuse rewrite on its user-module insert(s).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fire {
	/// At least one user `dict.insert` is rewritten to the transient in-place insert.
	Yes,
	/// A user `dict.insert` site exists but the analysis declines all of them (escape /
	/// not-dead-after / consumed-by-remove). `total >= 1 && reused == 0`.
	No,
}

struct Case {
	name: &'static str,
	fire: Fire,
	src: String,
}

// --------------------------------------------------------------------------
// Compile / run plumbing (in-memory; no temp files, no cwd mutation — both so
// the two emits stay independent under parallel test threads).
// --------------------------------------------------------------------------

/// Compile a single-module Pluma source string to lowered IR. Stdlib (`std.*`)
/// resolves from the baked-in registry, so an arbitrary root dir is fine.
fn lower_src(name: &str, src: &str) -> ir::IrProgram {
	let mut compiler = Compiler::for_root_dir(std::env::temp_dir());
	compiler.add_entry_module("main".into());
	compiler.set_module_source("main".into(), src.as_bytes().to_vec());
	compiler
		.check()
		.unwrap_or_else(|d| panic!("`{name}` failed to type-check: {}", join_diags(&d)));
	ir::lower(&compiler).unwrap_or_else(|m| panic!("`{name}` failed to lower: {m}"))
}

fn join_diags(diags: &[compiler::Diagnostic]) -> String {
	diags
		.iter()
		.map(|d| d.message.clone())
		.collect::<Vec<_>>()
		.join("; ")
}

/// Compile `src` with reuse on/off and run the artifact under V8, returning the
/// observable triple the run-suite also snapshots.
fn run(name: &str, src: &str, reuse: bool) -> (String, String, String) {
	let ir = lower_src(name, src);
	let bytes = wasm::emit_with_options(
		&ir,
		wasm::EmitOptions {
			reuse,
			..Default::default()
		},
	)
	.unwrap_or_else(|d| {
		panic!(
			"`{name}` failed to emit (reuse={reuse}): {}",
			d.0.join("; ")
		)
	});
	let cap = host::run_wasm_v8_captured(&bytes, &[]);
	(cap.status, cap.stdout, cap.stderr)
}

/// Classify the user-module `dict.insert` sites as the reuse lint would — replicating
/// the prefix of `wasm::emit`'s pipeline the pass runs after. Returns `(reused, total)`.
fn user_reuse_sites(name: &str, src: &str) -> (usize, usize) {
	let mut program = lower_src(name, src);
	ir::resolve::resolve_direct_calls(&mut program);
	ir::loopify::loopify(&mut program);
	ir::resolve::resolve_builtins(&mut program);
	let notes: Vec<_> = ir::reuse::report(&program)
		.into_iter()
		.filter(|n| !n.module.starts_with("std."))
		.collect();
	let reused = notes.iter().filter(|n| n.reused).count();
	(reused, notes.len())
}

// --------------------------------------------------------------------------
// The corpus.
// --------------------------------------------------------------------------

fn corpus() -> Vec<Case> {
	let mut cases = Vec::new();
	let mut add = |name: &'static str, fire: Fire, src: String| cases.push(Case { name, fire, src });

	// ---- FIRE: clean accumulators (exercise the transient insert path) ----

	// The headline shape: a tail-recursive `dict.insert` accumulator.
	add(
		"clean-accumulator",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 { m } else { build (dict.insert m (to-string i) (i * i)) (i - 1) }
}
def main = fun {
	let m = build (dict.empty ()) 12
	print (dict.size m)
	print (dict.lookup m "1" ?? -1)
	print (dict.lookup m "7" ?? -1)
	print (dict.lookup m "12" ?? -1)
	print (dict.lookup m "13" ?? -1)
}
"#
		.into(),
	);

	// A borrowing read (`dict.size`) inside the loop before the consume — a borrow must
	// not block reuse.
	add(
		"borrow-size-in-loop",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 {
		m
	} else {
		let s = dict.size m
		build (dict.insert m (to-string i) (i + s)) (i - 1)
	}
}
def main = fun {
	let m = build (dict.empty ()) 8
	print (dict.size m)
	print (dict.lookup m "8" ?? -1)
	print (dict.lookup m "1" ?? -1)
}
"#
		.into(),
	);

	// A borrowing `dict.lookup` of the accumulator inside the loop.
	add(
		"lookup-in-loop",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 {
		m
	} else {
		let prev = dict.lookup m (to-string i) ?? 0
		build (dict.insert m (to-string i) (i + prev)) (i - 1)
	}
}
def main = fun {
	let m = build (dict.empty ()) 6
	print (dict.size m)
	print (dict.lookup m "3" ?? -1)
}
"#
		.into(),
	);

	// Branching in the loop body: the insert stays unconditional (always consumes the
	// accumulator), but its value is chosen by an `if`-expression. Reuse still fires.
	add(
		"branching-value",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 {
		m
	} else {
		let v = if i > 5 { i * 100 } else { i }
		build (dict.insert m (to-string i) v) (i - 1)
	}
}
def main = fun {
	let m = build (dict.empty ()) 10
	print (dict.size m)
	print (dict.lookup m "6" ?? -1)
	print (dict.lookup m "3" ?? -1)
}
"#
		.into(),
	);

	// Two independent builds in one program: each call mints its own token, so a fresh
	// transient session must not disturb the other's result.
	add(
		"two-separate-builds",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 { m } else { build (dict.insert m (to-string i) (i * 10)) (i - 1) }
}
def main = fun {
	let a = build (dict.empty ()) 10
	let b = build (dict.empty ()) 4
	print (dict.size a)
	print (dict.size b)
	print (dict.lookup a "3" ?? -1)
	print (dict.lookup b "3" ?? -1)
	print (dict.lookup b "7" ?? -1)
}
"#
		.into(),
	);

	// Larger N drives the CHAMP trie deeper (multi-level descend + node splitting) under
	// transient mutation, not just the shallow root.
	add(
		"deep-trie",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 { m } else { build (dict.insert m (to-string i) (i * 3)) (i - 1) }
}
def main = fun {
	let m = build (dict.empty ()) 300
	print (dict.size m)
	print (dict.lookup m "1" ?? -1)
	print (dict.lookup m "150" ?? -1)
	print (dict.lookup m "299" ?? -1)
	print (dict.lookup m "300" ?? -1)
	print (dict.lookup m "301" ?? -1)
}
"#
		.into(),
	);

	// ---- FIRE, but the *input* is aliased: freeze / copy-on-write of foreign nodes ----

	// `build` fires (clean intra-function accumulator), but its caller keeps the dict it
	// was handed AND later reads it. The fresh token inside `build`'s second call makes
	// the snapshot's nodes foreign, so `tinsert` must copy them, leaving `snap` intact.
	add(
		"build-on-existing-snapshot",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 { m } else { build (dict.insert m (to-string i) i) (i - 1) }
}
def main = fun {
	let snap = build (dict.empty ()) 3
	let big = build snap 7
	# snap must stay {1,2,3}; big must be {1..7}.
	print (dict.size snap)
	print (dict.size big)
	print (dict.lookup snap "5" ?? -1)
	print (dict.lookup big "5" ?? -1)
	print (dict.lookup snap "2" ?? -1)
}
"#
		.into(),
	);

	// `build` fires, then the returned (now-frozen) dict is extended with a *persistent*
	// insert. The persistent path always copies, so the frozen base must be unchanged.
	add(
		"build-then-persistent-extend",
		Fire::Yes,
		r#"use std/dict
def build = fun m i {
	if i == 0 { m } else { build (dict.insert m (to-string i) i) (i - 1) }
}
def main = fun {
	let base = build (dict.empty ()) 6
	let more = dict.insert base "100" 999
	print (dict.size base)
	print (dict.size more)
	print (dict.lookup base "100" ?? -1)
	print (dict.lookup more "100" ?? -1)
}
"#
		.into(),
	);

	// ---- DECLINE: the dict or an intermediate escapes, so reuse must not fire ----

	// Each intermediate dict is stored in a list and read back later — every snapshot
	// must retain its own size (1, 2, 3, …). If the escape check regressed and reuse
	// fired here, the snapshots would alias one mutated dict and all show the final size.
	add(
		"snapshots-escape-into-list",
		Fire::No,
		r#"use std/dict
use std/list
def snapshots = fun m acc i {
	if i == 0 {
		acc
	} else {
		let m2 = dict.insert m (to-string i) i
		snapshots m2 [m2, ...acc] (i - 1)
	}
}
def main = fun {
	let snaps = snapshots (dict.empty ()) [] 5
	print (list.length snaps)
	list.each snaps (fun s { print (dict.size s) })
}
"#
		.into(),
	);

	// The accumulator is read again *after* the consume in the same iteration (not
	// dead-after): the pre-insert value is still needed, so reuse must decline. This is
	// also the suite's sharpest corruption witness — the read looks up the *just
	// inserted* key in the OLD dict via `dict.lookup` (which reads through the shared
	// CHAMP nodes, unlike the cached `dict.size`). Under value semantics the old dict
	// never has that key, so every lookup is `-1` (sum `-5`); if reuse wrongly fired and
	// mutated `m` in place, the freshly inserted value would leak back and the sum
	// diverges. (Validated by mutation-testing the harness against a defeated
	// dead-after check.)
	add(
		"read-after-insert",
		Fire::No,
		r#"use std/dict
def build = fun m acc i {
	if i == 0 {
		acc
	} else {
		let m2 = dict.insert m (to-string i) i
		let leaked = dict.lookup m (to-string i) ?? -1
		build m2 (acc + leaked) (i - 1)
	}
}
def main = fun {
	print (build (dict.empty ()) 0 5)
}
"#
		.into(),
	);

	// The intermediate dict flows into a tuple that is threaded through a helper — it
	// escapes into the `MakeTuple`, so reuse must decline.
	add(
		"escape-into-tuple",
		Fire::No,
		r#"use std/dict
def fst = fun p {
	let (a, _) = p
	a
}
def build = fun m i {
	if i == 0 {
		m
	} else {
		let m2 = dict.insert m (to-string i) i
		let pair = (m2, i)
		build (fst pair) (i - 1)
	}
}
def main = fun {
	let m = build (dict.empty ()) 5
	print (dict.size m)
	print (dict.lookup m "3" ?? -1)
}
"#
		.into(),
	);

	// A `dict.remove` in the same accumulator chain — no transient remove exists, so a
	// dict consumed by it stays persistent (the whole accumulator is ineligible).
	add(
		"remove-in-chain",
		Fire::No,
		r#"use std/dict
def build = fun m i {
	if i == 0 {
		m
	} else {
		let m2 = dict.insert m (to-string i) i
		let m3 = dict.remove m2 (to-string (i - 1))
		build m3 (i - 1)
	}
}
def main = fun {
	let m = build (dict.empty ()) 8
	print (dict.size m)
	print (dict.lookup m "8" ?? -1)
}
"#
		.into(),
	);

	cases
}

// --------------------------------------------------------------------------
// The tests.
// --------------------------------------------------------------------------

/// The core invariant: reuse never changes observable output. Runs every case both
/// ways and diffs. Collects all failures so one run reports the whole picture.
#[test]
fn reuse_is_observationally_transparent() {
	let mut failures = Vec::new();
	for case in corpus() {
		let on = run(case.name, &case.src, true);
		let off = run(case.name, &case.src, false);
		if on != off {
			failures.push(format!(
				"`{}`: reuse changed observable output\n    reuse on:  {:?}\n    reuse off: {:?}",
				case.name, on, off
			));
		} else if on.0 != "ok" {
			// Both sides agree but the program didn't run cleanly — a broken fixture,
			// not a soundness failure, but worth surfacing so the case still tests
			// something real.
			failures.push(format!(
				"`{}`: ran to a non-ok status (both ways): {:?}",
				case.name, on
			));
		}
	}
	assert!(
		failures.is_empty(),
		"reuse soundness violated:\n{}",
		failures.join("\n")
	);
}

/// The fire expectations hold: FIRE cases rewrite ≥1 user insert in place; DECLINE
/// cases have a user insert site that the analysis leaves persistent. This keeps the
/// corpus honest — a FIRE case that silently stopped firing (or a DECLINE case whose
/// insert vanished) would make its transparency check vacuous.
#[test]
fn fire_expectations_hold() {
	let mut failures = Vec::new();
	for case in corpus() {
		let (reused, total) = user_reuse_sites(case.name, &case.src);
		let ok = match case.fire {
			Fire::Yes => reused >= 1,
			Fire::No => total >= 1 && reused == 0,
		};
		if !ok {
			failures.push(format!(
				"`{}`: expected {:?}, got reused={reused} total={total}",
				case.name, case.fire
			));
		}
	}
	assert!(
		failures.is_empty(),
		"fire expectations violated:\n{}",
		failures.join("\n")
	);
}

/// Guard against the whole pass silently going dark (e.g. a pipeline-ordering change
/// that stops reuse from ever firing): the transparency test would then pass
/// vacuously. Require the corpus to actually exercise the transient path in several
/// places.
#[test]
fn suite_exercises_the_transient_path() {
	let firing = corpus()
		.iter()
		.filter(|c| user_reuse_sites(c.name, &c.src).0 >= 1)
		.count();
	assert!(
		firing >= 4,
		"expected several cases to fire the reuse rewrite, only {firing} did — the pass \
		 may have stopped firing, which would make the transparency test vacuous"
	);
}
