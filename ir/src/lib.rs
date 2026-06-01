// The Pluma mid-level IR.
//
// A target-independent intermediate representation that sits between the
// typed AST (produced by `compiler`) and code emission. The lowering pass
// (`lower`) performs all the elaboration that is independent of the eventual
// backend — closure conversion, dictionary passing, pattern compilation,
// `defer` edge insertion, async marking — so that each backend (today: the
// bytecode emitter in `codegen`; later: a WASM emitter) only has to translate
// the IR, not re-derive Pluma's semantics.
//
// Two design commitments matter for the WASM consumer:
//   * ANF — every intermediate result is a `Let`-bound `VarId`; call arguments
//     are atoms (`Atom`). Trivial to produce from a functional language and
//     trivial for any backend to emit from.
//   * Structured control flow is preserved (`If`/`Switch`/`Loop`, never
//     gotos) — WASM requires it, and the bytecode emitter is happy with it.
//
// The step-1 IR is deliberately minimal. Two passes from the full
// backend-neutral vision are deferred to step 2 (the WASM backend), because
// the bytecode VM needs neither:
//   * no `Repr`/unboxing — the VM is uniformly boxed, so the IR is too;
//   * no CPS/state-machine transform — async stays as the `Function::is_async`
//     flag plus an explicit `Rvalue::Await` node, riding the VM's existing
//     frame-snapshot runtime. Step 2 adds an IR->IR pass that rewrites these
//     into state machines (the snapshot trick can't port to WASM).
// The `is_async` flag and the `Await` node are the anticipated growth points.

pub mod copyprop;
pub mod cps;
pub mod inline;
pub mod lower;
pub mod mono;
pub mod repr;
pub mod resolve;
pub mod types;

pub use lower::lower;
pub use types::*;

/// The VM-path optimization sequence: inline small directly-called functions,
/// resolve indirect calls into direct ones, then eliminate the redundant copies
/// inlining introduced.
///
/// `resolve_direct_calls` rewrites `CallClosure(GlobalRef(g))` →
/// `Call(Callee::Function(fid))` for capture-free non-async targets and prunes
/// the orphaned global loads. This used to be skipped: the *stack* VM lowered a
/// `Call(Callee::Function)` to a fresh `MakeClosure` + `Call` (a per-call heap
/// allocation — a pessimization on every recursion). The **register VM** lowers
/// it to a `CallDirect` opcode — no global load, no closure value, no allocation
/// — so the pass flips from a pessimization to a win (and is the enabler the
/// step-2 monomorphization track needs). Run after inlining so it only sees the
/// calls inlining left behind.
///
/// `copyprop::eliminate_copies` then removes the `Let(dest, Use(ret))` copies the
/// inliner emits when binding a spliced call's return — codegen lowers each to a
/// `Move`, ~20% of executed opcodes on call-heavy code (M2a).
///
/// All behavior-neutral (validated by the conformance gate diffing VM output
/// against the deploy backends).
pub fn optimize(program: &mut IrProgram) {
	inline::inline(program);
	resolve::resolve_direct_calls(program);
	copyprop::eliminate_copies(program);
	// M5/M6: the unboxed-register substrate. The register VM keeps unboxed (`I64`)
	// values in a raw window; `insert_coercions` splices `Box`/`Unbox` at the repr
	// boundaries, and codegen reads the resulting reprs to emit raw-window opcodes.
	// The pipeline below is the *interprocedural* (M6) form: `monomorphize` fixes
	// each function's calling convention (eligible concrete self-recursive defs keep
	// unboxed `param_reprs`/`ret_repr`; everyone else reverts to all-`Boxed`), then
	// `Sigs::from_program` reflects it so eligible caller↔callee chains pass `i64`
	// directly with no per-call box/unbox. It is *correct* (M6 blocker fixed: see
	// `repr.rs`'s `self_ret` coercion and `codegen::reg`'s `uses_raw_registers` /
	// raw const-return — `tests/ir_mono` runs this exact path through the register VM).
	//
	// But it stays DISABLED, because measurement showed it is a net perf *loss for
	// the VM*: the VM's `Value::Int` is already inline-tagged (no heap box), so raw
	// arithmetic saves ~nothing, while unboxing adds `Box`/`Unbox` at every boundary
	// with the *many* ops that need a `Value` — a `MatchInt` subject (re-boxes a
	// monomorphized `fib`'s param every call: +150k `Box` on `fib(24)`), builtins,
	// and the F64/I32 coercions codegen can't use (they become identity `Move`s —
	// 38% of `float-bench`'s opcodes). The substrate's real payoff is the WASM
	// backend, whose boxes are genuine heap references. See notes/REGISTER_VM.md.
	const ENABLE_UNBOXED_REGISTERS: bool = false;
	if ENABLE_UNBOXED_REGISTERS {
		mono::monomorphize(program);
		let sigs = repr::Sigs::from_program(program);
		for f in &mut program.functions {
			if !f.is_async {
				f.var_reprs = repr::infer_reprs(f, &sigs);
				repr::insert_coercions(f, &sigs);
			}
		}
	}
}
