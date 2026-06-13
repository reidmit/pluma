// The Pluma mid-level IR.
//
// A target-independent intermediate representation that sits between the typed
// AST (produced by `compiler`) and code emission. The lowering pass (`lower`)
// performs all the elaboration that is independent of the backend — closure
// conversion, dictionary passing, pattern compilation, `defer` edge insertion,
// async marking — so the WasmGC backend (`wasm::emit`, Pluma's sole deploy
// backend) only has to translate the IR, not re-derive Pluma's semantics.
//
// Two design commitments matter for the backend:
//   * ANF — every intermediate result is a `Let`-bound `VarId`; call arguments
//     are atoms (`Atom`). Trivial to produce from a functional language and
//     trivial for the emitter to consume.
//   * Structured control flow is preserved (`If`/`Switch`/`Loop`, never gotos) —
//     WASM requires it.
//
// The backend-facing IR-to-IR passes live alongside lowering and are driven by
// `wasm::emit`: `resolve` (direct-call + builtin resolution), `repr` (the
// `Repr`/unboxing coercion pass — WasmGC boxes are genuine heap references, so
// making boxing explicit is a real win), and `cps` (the async state-machine
// transform that rewrites `Function::is_async` + `Rvalue::Await` into poll
// functions; the `is_async` flag and the `Await` node are its inputs).

pub mod cps;
pub mod loopify;
pub mod lower;
pub mod repr;
pub mod resolve;
pub mod reuse;
pub mod simplify;
pub mod types;

pub use lower::{lower, lower_entry, lower_tests};
pub use types::*;
