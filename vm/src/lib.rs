mod builtin;
mod instruction;
pub mod program;
pub mod stdlib;
mod task;
mod value;
mod vm;
mod wire;

pub use instruction::{ConstIdx, FuncIdx, GlobalIdx, Instruction, Offset, SlotIdx};
pub use program::{Function, Program};
pub use stdlib::{native_modules, NativeDef, NativeModule};
pub use value::{ClosureData, Value, VariantCtorData, VariantData};
pub use vm::{InputSource, OutputSink, RuntimeError, VM};
