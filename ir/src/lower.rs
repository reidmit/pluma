// Lowering: typed AST -> IR.
//
// This is where every backend-independent elaboration lives — the logic
// currently fused into `codegen/src/emit.rs`'s single AST->bytecode walk:
//   * identifier resolution (locals / captures / globals)
//   * closure conversion (explicit capture lists)
//   * dictionary elaboration (trait constraints -> dict params + GetDictMethod)
//   * pattern compilation (`when`/`if is` -> Switch + GetTag/GetPayload)
//   * `defer` edge insertion
//   * async marking (`Function::is_async` + `Await`)
//
// Phase 1.1 ports that elaboration here, function-by-function. For now this is
// a skeleton: the entry point exists so the seam is real and the crate
// compiles, but it is unimplemented and not yet called by `codegen`.

use crate::types::IrProgram;
use compiler::Compiler;

/// Lower a fully-analyzed program to IR.
///
/// Expects `compiler` to have completed `check()` (every module parsed and
/// analyzed, with inferred types attached to the AST).
///
/// Not yet implemented — see the phase plan in `IR.md`. The intended shape:
///   1. collect the enum table from every loaded module      (pre-pass)
///   2. reserve a `GlobalId` per top-level def / alias / instance (pre-pass)
///   3. lower each def body to a `Function` (the expr walk)
///   4. build the entry function and assemble the `IrProgram`
pub fn lower(compiler: &Compiler) -> IrProgram {
	let _ = compiler;
	todo!("phase 1.1: port AST->IR elaboration from codegen/src/emit.rs")
}
