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
// See `IR.md` at the repo root for the design-of-record and the phased plan.
// This crate is the step-1 scaffold: the type definitions and the `lower`
// entry point exist, but `lower` is not yet implemented or wired into
// `codegen` (phases 1.1-1.3).
//
// Two design commitments matter for the future WASM consumer:
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

pub mod lower;
pub mod types;

pub use lower::lower;
pub use types::*;
