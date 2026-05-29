// Codegen: mid-level IR → bytecode Program.
//
// The frontend produces a typed AST, which the `ir` crate lowers to a
// mid-level IR (`ir::lower`); this crate emits VM bytecode from that IR.
// `from_ir::emit` walks the IR collecting global slots (one per top-level
// def), builds the constants pool, emits a zero-arity "thunk function" per
// def whose body computes the def's value and Returns, then emits an entry
// thunk for `main` that calls the loaded value with () and returns the
// result.

pub mod from_ir;

pub use from_ir::emit as compile_from_ir;
