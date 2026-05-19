mod doc;
mod visitor;

use compiler::{Diagnostic, Module};
use std::path::PathBuf;

pub const DEFAULT_LINE_WIDTH: usize = 100;

// Parse `source` as Pluma and return its canonical formatted form. On parse
// failure, returns the diagnostics produced during parsing.
pub fn format_source(source: &[u8]) -> Result<String, Vec<Diagnostic>> {
	format_source_with_width(source, DEFAULT_LINE_WIDTH)
}

pub fn format_source_with_width(source: &[u8], width: usize) -> Result<String, Vec<Diagnostic>> {
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	let mut module = Module::new("<format>".to_string(), PathBuf::from("<format>"));
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);

	if diagnostics.iter().any(|d| d.is_error()) {
		return Err(diagnostics);
	}

	let ast = module.ast.as_ref().expect("parser populated ast");
	let formatter = visitor::Formatter::new(&module.comments);
	let doc = formatter.format_module(ast);

	let raw = doc::render(&doc, width);
	let mut out = strip_trailing_whitespace(&raw);
	if !out.ends_with('\n') {
		out.push('\n');
	}
	Ok(out)
}

fn strip_trailing_whitespace(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for (i, line) in s.split('\n').enumerate() {
		if i > 0 {
			out.push('\n');
		}
		let trimmed = line.trim_end();
		out.push_str(trimmed);
	}
	out
}
