use compiler::{Compiler, Diagnostic, Module};
use std::path::Path;

pub struct AnalysisResult {
	// The analyzed entry module. Present even when diagnostics is non-empty
	// — the analyzer attaches the inferred types it could resolve, so hover
	// still has something useful to show on partial failures.
	pub module: Option<Module>,
	pub diagnostics: Vec<Diagnostic>,
}

// Run the full compiler pipeline (parse + analyze) against an in-memory
// document, with imports resolved from disk relative to the document's
// containing project. Returns the analyzed entry module and any
// diagnostics produced along the way.
pub fn analyze_document(path: &Path, source: Vec<u8>) -> AnalysisResult {
	let entry_path_str = match path.to_str() {
		Some(s) => s.to_string(),
		None => {
			return AnalysisResult {
				module: None,
				diagnostics: vec![Diagnostic::error("document path is not valid UTF-8")],
			};
		}
	};

	let mut compiler = match Compiler::from_entry_path(entry_path_str) {
		Ok(c) => c,
		Err(diagnostics) => {
			return AnalysisResult {
				module: None,
				diagnostics,
			};
		}
	};

	vm::stdlib::register_compiler(&mut compiler);

	let entry_name = compiler.entry_modules.first().cloned().unwrap_or_default();
	compiler.set_module_source(entry_name.clone(), source);

	let diagnostics = compiler.check().err().unwrap_or_default();
	let module = compiler.modules.remove(&entry_name);

	AnalysisResult {
		module,
		diagnostics,
	}
}
