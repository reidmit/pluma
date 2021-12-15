use ast::*;
use diagnostics::*;
use parser::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use visitor::*;

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

	pub fn parse(&mut self) -> Result<(), Vec<Diagnostic>> {
		let mut diagnostics = Vec::new();

		match fs::read(&self.module_path) {
			Ok(bytes) => self.build_ast(bytes, &mut diagnostics),
			Err(err) => diagnostics.push(
				Diagnostic::error(err)
					.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
			),
		}

		if diagnostics.is_empty() {
			return Ok(());
		}

		return Err(diagnostics);
	}

	pub fn did_parse(&self) -> bool {
		self.ast.is_some()
	}

	pub fn get_line_for_position(&self, pos: Position) -> usize {
		let mut line = 1;

		for break_start in &self.line_break_starts {
			if break_start >= &pos.0 && break_start <= &pos.1 {
				return line;
			}

			line += 1
		}

		line
	}

	pub fn get_comment_for_line(&self, line: usize) -> Option<&String> {
		self.comments.get(&line)
	}

	pub fn traverse<V: Visitor>(&self, visitor: &mut V) {
		if let Some(ast) = &self.ast {
			ast.traverse(visitor)
		}
	}

	pub fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
		if let Some(ast) = &mut self.ast {
			ast.traverse_mut(visitor)
		}
	}

	fn build_ast(&mut self, bytes: Vec<u8>, diagnostics: &mut Vec<Diagnostic>) {
		let tokenizer = Tokenizer::from_source(&bytes);

		let (ast, comment_data, errors) = Parser::new(&bytes, tokenizer).parse_module();

		self.ast = Some(ast);

		let (comments, line_break_starts) = comment_data;
		self.comments = comments;
		self.line_break_starts = line_break_starts;

		if !errors.is_empty() {
			for err in errors {
				diagnostics.push(
					Diagnostic::error(err)
						.with_pos(err.pos)
						.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
				);
			}
		}
	}
}
