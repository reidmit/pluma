// The JavaScript backend: lowers the shared `ir::IrProgram` to a self-contained
// JS module for the browser/client target. A sibling of `codegen` (VM bytecode)
// and `wasm` (WasmGC), consuming the same IR.
//
// `emit(&program)` returns the JS source: a runtime preamble (`runtime.js`) +
// the compiled functions + the global table + the entry call. Run it under node
// or in a browser. See `emit.rs` for the lowering.

mod emit;

pub use emit::emit;
