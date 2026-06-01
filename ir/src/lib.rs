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
/// then resolve indirect calls to statically-known top-level functions into
/// direct calls.
///
/// `resolve_direct_calls` rewrites `CallClosure(GlobalRef(g))` →
/// `Call(Callee::Function(fid))` for capture-free non-async targets and prunes
/// the orphaned global loads. This used to be skipped: the *stack* VM lowered a
/// `Call(Callee::Function)` to a fresh `MakeClosure` + `Call` (a per-call heap
/// allocation — a pessimization on every recursion). The **register VM** lowers
/// it to a `CallDirect` opcode — no global load, no closure value, no allocation
/// — so the pass flips from a pessimization to a win (and is the enabler the
/// step-2 monomorphization track needs). Run after inlining so it only sees the
/// calls inlining left behind. Behavior-neutral (validated by the conformance
/// gate diffing VM output against the deploy backends).
pub fn optimize(program: &mut IrProgram) {
	inline::inline(program);
	resolve::resolve_direct_calls(program);
	// M5: repr coercions so the register VM can keep unboxed (`I64`) values in a
	// raw window. Inserts `Box`/`Unbox` at repr boundaries; codegen reads the
	// resulting reprs and emits raw-window opcodes. Uniform `Sigs` for now
	// (interprocedural unboxing across call boundaries is M6). Async functions
	// are left boxed — the `drive_step` snapshot stays boxed-only.
	// Repr coercions (the unboxed-register substrate — M5) are implemented and
	// validated (`tests/ir_repr`, `tests/ir_mono`; see notes/REGISTER_VM.md) but
	// DISABLED in the VM pipeline: intra-function unboxing *alone* is a net perf
	// loss, because boxed call boundaries cost a Box/Unbox per call that outweighs
	// the arithmetic win on the call-heavy corpus. The payoff needs M6 (unboxed
	// call boundaries via monomorphization), which is blocked on a `mono` /
	// `repr::Sigs::from_program` `ret_repr` inconsistency the register VM is the
	// first backend to surface. Flip this to `true` once M6 lands.
	const ENABLE_UNBOXED_REGISTERS: bool = false;
	if ENABLE_UNBOXED_REGISTERS {
		let sigs = repr::Sigs::uniform();
		for f in &mut program.functions {
			if !f.is_async {
				f.var_reprs = repr::infer_reprs(f, &sigs);
				repr::insert_coercions(f, &sigs);
			}
		}
	}
}
