// Codegen: typed AST → bytecode Program.
//
// The lowering pipeline:
//   1. Walk the entire program collecting global slots (one per top-level
//      def, in every loaded module). Also collect enum variant info.
//   2. Build the constants pool: every distinct string and regex source
//      gets one slot.
//   3. For each top-level def, emit a "thunk function" (zero-arity, no
//      captures) whose body computes the def's value and Returns. The
//      thunk's FuncIdx goes into the global's Pending state.
//   4. Inside thunks (and any nested funs), walk expressions emitting
//      instructions. Identifier resolution distinguishes locals, captures,
//      and globals.
//   5. The entry function is a thunk for `main` that finishes by calling
//      the loaded value with () and returns the result.

mod emit;
pub mod from_ir;

pub use emit::compile;
pub use from_ir::emit as compile_from_ir;
