// Codegen: mid-level IR → register-VM Program.
//
// The frontend produces a typed AST, which the `ir` crate lowers to a mid-level
// IR (`ir::lower`); this crate emits register-VM bytecode from that IR.
// `reg::emit` walks the IR collecting global slots (one per top-level def),
// builds the constants/reg-list pools, emits a thunk function per def, and an
// entry thunk for `main`. See notes/REGISTER_VM.md.

pub mod reg;

pub use reg::emit as compile_from_ir;
