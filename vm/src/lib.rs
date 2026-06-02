mod builtin;
mod net;
// `program` now holds only the shared `GlobalSlot`; the program/function shapes
// live in `reg` (the register VM — see notes/REGISTER_VM.md).
pub mod program;
pub mod reg;
pub mod stdlib;
mod task;
mod value;
mod vm;
mod wire;

// The register VM is the engine: `Program`/`Function` are the register shapes.
pub use program::GlobalSlot;
pub use reg::{ConstIdx, FuncIdx, Function, GlobalIdx, Instruction, Offset, Program, Reg};
pub use stdlib::{NativeDef, NativeModule, native_modules};
pub use value::{ClosureData, Value, VariantCtorData, VariantData};
pub use vm::{InputSource, OutputSink, RuntimeError, VM};
