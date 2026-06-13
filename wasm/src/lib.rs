//! WASM (WasmGC) backend — the consumer of `ir::IrProgram` that lowers the IR
//! into a WasmGC module.
//!
//! `emit(&IrProgram)` runs the WASM-readiness pipeline —
//! direct-call resolution + Repr coercion (uniform-boxed; monomorphization is a
//! follow-on) — then dead-code-eliminates to the entry-reachable functions and
//! emits a WasmGC module. The reachability prune is load-bearing: even a
//! `print (1 + 2)` program lowers the whole prelude, but almost none of it is
//! reachable, so the emitter only ever sees the handful of functions a program
//! actually runs.
//!
//! An unsupported IR node becomes a `Diagnostic` rather than a silent
//! miscompile; the `tests/run` snapshot suite is the regression guard.

use ir::{Block, Callee, GlobalInit, IrProgram, PreEval, Rvalue, StmtKind};
use std::collections::{HashMap, HashSet};

mod async_lower;
mod emit;
mod helpers;
mod module;
mod mono;
mod opt;
mod runtime;
mod scan;
mod types;
mod util;

pub use diag::Diagnostics;
pub use opt::{OptLevel, optimize};

mod diag {
	/// An accumulated emit failure, attributed where possible. Plural by design
	/// (mirrors the compiler's `Vec<Diagnostic>` discipline): emission collects
	/// every unsupported node so a coverage pass can enumerate the gaps.
	#[derive(Debug, Default)]
	pub struct Diagnostics(pub Vec<String>);

	impl Diagnostics {
		pub fn push(&mut self, msg: impl Into<String>) {
			self.0.push(msg.into());
		}
		pub fn is_empty(&self) -> bool {
			self.0.is_empty()
		}
	}
}

/// Knobs on the emit pipeline. Defaults match `pluma run`/`pluma build`; the only
/// reason to deviate is the soundness harness (`tests/soundness.rs`), which emits a
/// program twice — once with `reuse` on, once off — and asserts byte-identical
/// observable output. A thread-safe options value (not a process-global env var) so
/// the two emits can run concurrently under the parallel test harness without racing.
#[derive(Clone, Copy, Debug)]
pub struct EmitOptions {
	/// Run the opportunistic in-place reuse pass (`ir::reuse`). On by default; the
	/// pass is sound, so turning it off only forgoes the perf win — the persistent
	/// baseline it falls back to is the observational oracle the harness diffs against.
	pub reuse: bool,
	/// Emit for the browser target (`pluma build --web`): use the long-lived
	/// command runtime entry (`__browser_entry`) instead of the run-to-completion
	/// `__task_entry`, export `__browser_resume`, and wire the `__dom_dispatch` pump tail.
	pub browser: bool,
}

impl Default for EmitOptions {
	fn default() -> Self {
		EmitOptions {
			reuse: true,
			browser: false,
		}
	}
}

/// Lower an `IrProgram` to a WasmGC module. Returns the encoded `.wasm` bytes, or
/// the accumulated diagnostics if any reachable construct isn't yet supported.
pub fn emit(program: &IrProgram) -> Result<Vec<u8>, Diagnostics> {
	emit_with_options(program, EmitOptions::default())
}

/// `emit`, with the pipeline knobs exposed (see [`EmitOptions`]).
pub fn emit_with_options(program: &IrProgram, opts: EmitOptions) -> Result<Vec<u8>, Diagnostics> {
	// 1. WASM-readiness passes specific to emission. Direct-call resolution exposes
	//    statically-known callees (and lets the entry->main bootstrap collapse to
	//    a direct call); coercion makes boxing explicit so the emitter reads
	//    i64/f64/i32 vs GC-ref straight off `var_reprs`.
	let mut p = program.clone();
	// Async lowering FIRST: cps-transform awaiting functions into poll fns and
	// rewrite their bodies into `$task` constructors (so the Await-bodies never
	// reach the emitter). Every program is driven by the scheduler — a fully sync
	// `main` just returns a plain value the entry wrapper hands straight back.
	async_lower::lower(&mut p);
	ir::resolve::resolve_direct_calls(&mut p);
	// Turn self-tail-recursion into a `Loop` over the params. Behavior-neutral; the
	// enabler for intra-function reuse analysis. Needs
	// `TailCallDirect` (so after direct-call resolution) and must precede the repr
	// pass (so reassigned-param / result locals get reprs).
	ir::loopify::loopify(&mut p);
	// Resolve builtin-global calls to typed `Call(Callee::Builtin(tag, ret))` nodes,
	// threading each builtin's declared return repr onto the call so the coercion
	// pass below can read a scalar-returning builtin (`bytes-get`, …) unboxed.
	ir::resolve::resolve_builtins(&mut p);
	// Opportunistic in-place reuse: rewrite a proven-unique `dict.insert` accumulator
	// (the `loopify`'d loop carry) to the transient in-place insert. Sound-only; sees
	// the resolved `dict-insert` builtin call, so it runs after `resolve_builtins`, and
	// mints a token local, so before the repr pass. Gated so the
	// soundness harness can emit the persistent baseline (reuse off) for its differential.
	if opts.reuse {
		ir::reuse::reuse(&mut p);
	}
	// Record-shape monomorphization: clone record-param functions per call-site
	// shape so the clone reads its param by `struct.get` (and the caller passes it
	// nominal). Returns the per-clone param shapes the emitter consumes. Runs before
	// the repr pass so clones get their `var_reprs`/coercions too.
	let mut param_shapes = mono::specialize_record_shapes(&mut p);
	// Merge the substitution-driven engine's specialized-function param shapes
	// (produced during lowering — a generic def re-lowered under a closed type
	// substitution, whose record params hold a `$shapeN`). Distinct `FuncId`s from
	// `mono`'s call-site-shape clones, so the maps don't collide.
	for (fid, shapes) in &p.param_shapes {
		param_shapes.entry(*fid).or_insert_with(|| shapes.clone());
	}
	// Numeric monomorphization: an eligible direct-only function keeps its recorded
	// param/return reprs (so a concrete `fib : int -> int` takes an i64 param and
	// returns an i64, with no box/unbox at the call boundary); every function that
	// escapes as a first-class value is forced back to the uniform-boxed contract so
	// its indirect calls still agree. `Sigs::from_program` then reads the filtered
	// signatures, and the coercion pass makes each direct call site match.
	monomorphize_signatures(&mut p);
	let sigs = ir::repr::Sigs::from_program(&p);
	let extra_nominal = p.extra_nominal.clone();
	for (fid, f) in p.functions.iter_mut().enumerate() {
		// The per-function nominal-record map (same one the emitter recomputes), so
		// the coercion pass reads/stores each unboxed (`F64`) record field at its slot
		// repr instead of boxing — keeping repr inference in step with emission.
		let nominal = scan::compute_nominal(f, fid as u32, &param_shapes, &extra_nominal);
		f.var_reprs = ir::repr::infer_reprs(f, &sigs, &nominal);
		ir::repr::insert_coercions(f, &sigs, &nominal);
	}

	// 2. Dead-code elimination: only functions/globals reachable from the entry.
	let reach = Reach::compute(&p);

	// 3. Build and encode the module.
	let mut diags = Diagnostics::default();
	let bytes = module::Module::build(
		&p,
		&reach,
		&param_shapes,
		&extra_nominal,
		opts.browser,
		&mut diags,
	);
	if diags.is_empty() {
		Ok(bytes)
	} else {
		Err(diags)
	}
}

// --------------------------------------------------------------------------
// Reachability (DCE).
// --------------------------------------------------------------------------

/// The set of functions and globals reachable from the program entry, plus a
/// dense `FuncId -> wasm-defined-index` numbering over the reachable functions.
pub(crate) struct Reach {
	/// Reachable global ids (which globals' thunks/values to realize).
	pub globals: HashSet<u32>,
	/// Reachable function ids in dense order (the order they're emitted).
	pub order: Vec<u32>,
}

impl Reach {
	fn compute(p: &IrProgram) -> Self {
		let mut funcs = HashSet::new();
		let mut globals = HashSet::new();
		let mut order = Vec::new();
		let mut stack = vec![p.entry.0];
		let mut gstack: Vec<u32> = Vec::new();
		while let Some(fi) = stack.pop() {
			if !funcs.insert(fi) {
				continue;
			}
			order.push(fi);
			scan_block(&p.functions[fi as usize].body, &mut stack, &mut gstack);
			while let Some(gi) = gstack.pop() {
				if !globals.insert(gi) {
					continue;
				}
				if let GlobalInit::Thunk(f) = &p.globals[gi as usize] {
					stack.push(f.0);
				}
			}
		}
		let _ = funcs;
		Self { globals, order }
	}
}

// --------------------------------------------------------------------------
// Numeric-signature monomorphization. A function keeps its recorded
// (`param_reprs`, `ret_repr`) unboxed signature only when *every* use of it is a
// resolved direct call — i.e. it never escapes as a first-class closure value, so
// no indirect (uniform-boxed) call site can disagree with the unboxed convention.
// A function escapes iff a *reachable* function builds a closure over it: its
// capture-free backing thunk (`MakeClosure(fid, [])`) is itself reachable exactly
// when the function's global is loaded somewhere (passed to `map`, stored, …);
// a direct-only function's thunk is dead after `resolve_direct_calls` pruned the
// loads, so it never appears. The program entry is excluded too — the synthetic
// `__task_entry` wrapper calls it with the uniform-boxed convention.
// --------------------------------------------------------------------------

fn monomorphize_signatures(p: &mut IrProgram) {
	let reach = Reach::compute(p);
	let mut escaped: HashSet<u32> = HashSet::new();
	for &fid in &reach.order {
		collect_closure_targets(&p.functions[fid as usize].body, &mut escaped);
	}
	let entry = p.entry.0;
	for (i, f) in p.functions.iter_mut().enumerate() {
		let eligible = i as u32 != entry && !escaped.contains(&(i as u32));
		if !eligible {
			f.param_reprs = vec![ir::Repr::Boxed; f.params.len()];
			f.ret_repr = ir::Repr::Boxed;
		}
	}
}

/// Collect every `FuncId` a block builds a closure over (`MakeClosure`) — the
/// functions that escape as first-class values.
fn collect_closure_targets(b: &Block, out: &mut HashSet<u32>) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Rvalue::MakeClosure(f, _) = rv {
					out.insert(f.0);
				}
			}
			StmtKind::If(_, t, e) => {
				collect_closure_targets(t, out);
				collect_closure_targets(e, out);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_closure_targets(b, out);
				}
				collect_closure_targets(default, out);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_closure_targets(&a.body, out);
				}
			}
			StmtKind::Loop(b) => collect_closure_targets(b, out),
			_ => {}
		}
	}
}

fn scan_rvalue(rv: &Rvalue, fns: &mut Vec<u32>, gs: &mut Vec<u32>) {
	match rv {
		Rvalue::Call(Callee::Function(f), _)
		| Rvalue::TailCallDirect(f, _)
		| Rvalue::MakeClosure(f, _) => fns.push(f.0),
		Rvalue::Call(Callee::Global(g), _) | Rvalue::GlobalRef(g) => gs.push(g.0),
		_ => {}
	}
}

fn scan_block(b: &Block, fns: &mut Vec<u32>, gs: &mut Vec<u32>) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => scan_rvalue(rv, fns, gs),
			StmtKind::If(_, t, e) => {
				scan_block(t, fns, gs);
				scan_block(e, fns, gs);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					scan_block(b, fns, gs);
				}
				scan_block(default, fns, gs);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					scan_block(&a.body, fns, gs);
				}
			}
			StmtKind::Loop(b) => scan_block(b, fns, gs),
			_ => {}
		}
	}
}

// --------------------------------------------------------------------------
// Builtin globals. A `PreEvaluated(Builtin(tag))` global referenced as a call
// target is a host primitive. We map each such global to its tag so the emitter
// can turn a `CallClosure`/`TailCall` on it into a host-import call.
// --------------------------------------------------------------------------

pub(crate) fn builtin_globals(p: &IrProgram) -> HashMap<u32, String> {
	let mut m = HashMap::new();
	for (i, g) in p.globals.iter().enumerate() {
		if let GlobalInit::PreEvaluated(PreEval::Builtin(tag, _)) = g {
			m.insert(i as u32, tag.clone());
		}
	}
	m
}
