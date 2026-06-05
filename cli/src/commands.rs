//! CLI subcommand implementations — one module per `pluma` command. Each exposes
//! a single entry point invoked by the dispatcher in `main`; shared infrastructure
//! (diagnostics printing, the browser bundle) lives in the top-level modules.

pub(crate) mod build;
pub(crate) mod dev;
pub(crate) mod format;
pub(crate) mod run;
pub(crate) mod test;

#[cfg(debug_assertions)]
pub(crate) mod analyze;
#[cfg(debug_assertions)]
pub(crate) mod tokenize;
