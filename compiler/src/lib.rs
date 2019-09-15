pub const DEFAULT_ENTRY_FILE: &str = "main.pa";
pub const FILE_EXTENSION: &str = "pa";
pub const VERSION: &str = "0.1.0";

pub mod compiler;
pub mod errors;
pub mod error_formatter;

mod ast;
mod fs;
mod import_chain;
mod macros;
mod module;
mod parser;
mod tokenizer;
mod tokens;