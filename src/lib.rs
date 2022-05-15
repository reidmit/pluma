mod analyzer;
mod ast;
mod binding;
mod colors;
mod compiler;
mod diagnostic;
mod errors;
mod expr_type;
mod intrinsics;
mod module;
mod parser;
mod tokenizer;
mod tokens;

pub use compiler::*;
pub use diagnostic::*;

pub const BINARY_NAME: &str = "pencil";
pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const DEFAULT_ENTRY_FILE: &str = "main.pa";
pub const FILE_EXTENSION: &str = "pa";
pub const VERSION: &str = "0.1.0";
