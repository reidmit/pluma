mod analyzer;
mod ast;
mod binding;
mod compiler;
mod diagnostic;
mod errors;
mod module;
mod parser;
mod tokenizer;
mod tokens;
mod types;

pub use compiler::*;
pub use diagnostic::*;

pub const VERSION: &str = "0.1.0";
pub const BINARY_NAME: &str = "pluma";
pub const LANGUAGE_NAME: &str = "Pluma";

pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const DEFAULT_ENTRY_FILE: &str = "main.pa";
pub const FILE_EXTENSION: &str = "pa";
