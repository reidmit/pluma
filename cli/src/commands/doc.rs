//! `pluma doc <module>` — generate documentation for a module from its source.
//!
//! Analyzes the named module (the stdlib is baked in, so `std/list` resolves
//! without anything on disk) and emits a self-contained Pluma data module
//! describing its public surface. A docs site `use`s that module and renders
//! it, so the docs come straight from the source — signatures from the
//! compiler's own inferred types, prose from the `#` doc comments.
//!
//! The special target `std` documents every baked-in stdlib module at once.

use std::path::PathBuf;

use compiler::docs::{self, ModuleDoc};
use compiler::{Compiler, lookup_stdlib_source, stdlib_sources};

use crate::printing::*;

pub(crate) fn doc_command(module: String, out: Option<String>) {
	let models = if module == "std" {
		document_all_stdlib()
	} else {
		vec![document_one(&module).unwrap_or_else(|diagnostics| {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		})]
	};

	let source = docs::to_pluma_source(&models);
	match out {
		Some(path) => match std::fs::write(&path, source) {
			Ok(()) => eprintln!("wrote docs for {} module(s) to {path}", models.len()),
			Err(e) => {
				eprintln!("pluma doc: could not write {path}: {e}");
				std::process::exit(1);
			}
		},
		None => print!("{source}"),
	}
}

// Every baked-in stdlib module, analyzed independently so one module that fails
// to check (e.g. a platform-gated surface) is skipped with a note rather than
// aborting the whole run. Modules with no public surface are dropped silently.
fn document_all_stdlib() -> Vec<ModuleDoc> {
	let mut names: Vec<&str> = stdlib_sources().iter().map(|(n, _)| *n).collect();
	names.sort_unstable();

	let mut models = Vec::new();
	for name in names {
		match document_one(name) {
			Ok(model) if !model.items.is_empty() => models.push(model),
			Ok(_) => {}
			Err(_) => eprintln!("pluma doc: skipped `{name}` (did not analyze)"),
		}
	}
	models
}

// Analyze an in-memory entry that imports the target so its source is parsed and
// type-checked and lands in the module cache — no file needed.
fn document_one(module: &str) -> Result<ModuleDoc, Vec<compiler::Diagnostic>> {
	let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
	let mut compiler = Compiler::for_root_dir(root);
	let entry = "__pluma_doc_entry__".to_string();
	compiler.add_entry_module(entry.clone());
	compiler.set_module_source(entry, format!("use {module}\n").into_bytes());
	compiler.check()?;

	let analyzed = compiler.modules.get(module).ok_or_else(Vec::new)?;
	let source = lookup_stdlib_source(module).unwrap_or("");
	Ok(docs::extract(analyzed, module, source))
}
