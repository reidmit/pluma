#![allow(unused_variables)]
#![allow(dead_code)]

pub const LANG_NAME: &str = "pluma";
pub const LANG_NAME_UPPER: &str = "Pluma";
pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const FILE_EXTENSION: &str = "pa";
pub const VERSION: &str = "0.1.0";

pub mod compiler;
pub mod diagnostics;
pub mod errors;
pub mod parser;
pub mod tokenizer;

mod ast;
mod dependency_graph;
mod module;
mod tokens;
