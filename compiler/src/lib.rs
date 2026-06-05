mod analyzer;
pub mod ast;
mod binding;
mod compiler;
mod diagnostic;
mod errors;
mod location;
mod module;
mod parser;
mod platform;
mod reachability;
mod render;
pub mod rpc;
mod stdlib;
mod suggest;
mod tokenizer;
mod tokens;
pub mod types;

pub use compiler::*;
pub use diagnostic::*;
pub use location::*;
pub use module::{EnumExport, Module, ModuleExports, ValueConstraintExport};
pub use platform::{Target, gate};
pub use render::{Palette, render_diagnostics};
pub use stdlib::{lookup_stdlib_source, stdlib_sources};
pub use tokenizer::*;
pub use tokens::Token;

// `0.1.0-<short-sha>` in a git checkout, bare `0.1.0` from a tarball — baked by build.rs.
pub const VERSION: &str = env!("PLUMA_VERSION");
pub const BINARY_NAME: &str = "pluma";
pub const LANGUAGE_NAME: &str = "Pluma";

pub const DEFAULT_ENTRY_MODULE_NAME: &str = "main";
pub const DEFAULT_ENTRY_FILE: &str = "main.pa";
pub const FILE_EXTENSION: &str = "pa";
