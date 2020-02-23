pub const LANG_NAME: &str = "pluma";
pub const LANG_NAME_UPPER: &str = "Pluma";
pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const FILE_EXTENSION: &str = "pa";
pub const VERSION: &str = "0.1.0";

pub mod analyzer;
pub mod compiler;
pub mod error_formatter;
pub mod errors;
pub mod parser2;
pub mod tokenizer;

mod ast;
mod ast2;
mod dependency_graph;
mod fs;
mod macros;
mod module;
mod scope;
mod tokens;
