use crate::ast::*;
use crate::diagnostic::*;
use crate::parser::*;
use crate::tokenizer::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Module {
	pub module_name: String,
	pub module_path: PathBuf,
	pub ast: Option<ModuleNode>,
	pub comments: HashMap<usize, String>,
	pub line_break_starts: Vec<usize>,
}

impl Module {
	pub fn new(module_name: String, module_path: PathBuf) -> Module {
		Module {
			module_name,
			module_path,
			ast: None,
			comments: HashMap::new(),
			line_break_starts: Vec::new(),
		}
	}

	pub fn parse(&mut self, diagnostics: &mut Vec<Diagnostic>) {
		match fs::read(&self.module_path) {
			Ok(bytes) => self.build_ast(bytes, diagnostics),
			Err(err) => diagnostics.push(
				Diagnostic::error(err)
					.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
			),
		}
	}

	pub fn did_parse(&self) -> bool {
		self.ast.is_some()
	}

	pub fn get_line_for_span(&self, span: Span) -> usize {
		let mut line = 1;

		for break_start in &self.line_break_starts {
			if break_start >= &span.0 && break_start <= &span.1 {
				return line;
			}

			line += 1
		}

		line
	}

	pub fn get_comment_for_line(&self, line: usize) -> Option<&String> {
		self.comments.get(&line)
	}

	fn build_ast(&mut self, bytes: Vec<u8>, diagnostics: &mut Vec<Diagnostic>) {
		let tokenizer = Tokenizer::from_source(&bytes);

		let (ast, comments, errors) = Parser::new(&bytes, tokenizer).parse_module();

		self.ast = Some(ast);
		self.comments = comments;

		for err in errors {
			diagnostics.push(
				Diagnostic::error(err)
					.with_span(err.span)
					.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
			);
		}
	}
}
