pub const LANG_NAME: &str = "pluma";
pub const LANG_NAME_UPPER: &str = "Pluma";
pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const FILE_EXTENSION: &str = "pa";
pub const VERSION: &str = "0.1.0";

pub mod compiler;
pub mod errors;
pub mod error_formatter;
pub mod parser;
pub mod tokenizer;

mod analyzer;
mod ast;
mod dependency_graph;
mod fs;
mod macros;
mod module;
mod tokens;