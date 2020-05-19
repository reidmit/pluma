#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

pub const LANG_NAME: &str = "pluma";
pub const LANG_NAME_UPPER: &str = "Pluma";
pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const FILE_EXTENSION: &str = "pa";
pub const VERSION: &str = "0.1.0";

pub mod analyzer;
pub mod compiler;
pub mod diagnostics;
pub mod parser;
pub mod tokenizer;

mod analysis_error;
mod ast;
mod dependency_graph;
mod import_error;
mod module;
mod parse_error;
mod scope;
mod tokens;
mod traverse;
mod type_collector;
mod types;
mod usage_error;
mod visitor;
