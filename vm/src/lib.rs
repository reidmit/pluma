mod builtin;
mod eval;
mod instruction;
pub mod program;
pub mod stdlib;
mod value;
mod vm;

pub use builtin::Builtin;
pub use instruction::{ConstIdx, FuncIdx, GlobalIdx, Instruction, Offset, SlotIdx};
pub use program::{Function, Program};
pub use stdlib::{native_modules, NativeDef, NativeModule};
pub use value::{ClosureData, RegexData, Value, VariantCtorData, VariantData};
pub use vm::{OutputSink, RuntimeError, VM};
