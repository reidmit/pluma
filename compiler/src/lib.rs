// TODO remove these, these are just for testing
#![allow(dead_code)]
#![allow(unused_imports)]

pub mod compiler;
pub mod errors;

mod ast;
mod fs;
mod macros;
mod module;
mod parser;
mod tokenizer;
mod tokens;