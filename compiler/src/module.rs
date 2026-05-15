use crate::ast::*;
use crate::diagnostic::*;
use crate::parser::*;
use crate::tokenizer::*;
use crate::types::*;
use crate::Token;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// What a module exposes to anyone that `use`s it. Populated by the Analyzer
// after type inference.
//
// `values` are top-level value defs (including alias constructor functions).
// `aliases` are alias type defs (name -> the resolved underlying type).
// `enums` are enum type defs (name -> ordered list of variants).
//
// Types inside this struct carry the defining module's bare enum names (e.g.
// `Type::Enum("color")`); the importing analyzer qualifies them with the
// defining module's fully-qualified name when pulling them in.
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone, Default)]
pub struct ModuleExports {
	pub values: HashMap<String, Type>,
	pub aliases: HashMap<String, Type>,
	pub enums: HashMap<String, Vec<(String, Vec<Type>)>>,
}

pub struct Module {
	pub module_name: String,
	pub module_path: PathBuf,
	pub ast: Option<ModuleNode>,
	pub comments: HashMap<usize, String>,
	pub line_break_starts: Vec<usize>,
	// Top-level definitions exposed to importing modules. `None` means not
	// yet analyzed.
	pub exports: Option<ModuleExports>,
}

impl Module {
	pub fn new(module_name: String, module_path: PathBuf) -> Module {
		Module {
			module_name,
			module_path,
			ast: None,
			comments: HashMap::new(),
			line_break_starts: Vec::new(),
			exports: None,
		}
	}

	pub fn tokenize(&mut self, diagnostics: &mut Vec<Diagnostic>) -> Vec<Token> {
		match fs::read(&self.module_path) {
			Ok(bytes) => {
				let tokenizer = Tokenizer::from_source(&bytes);
				let tokens = tokenizer.collect();
				tokens
			}
			Err(err) => {
				diagnostics.push(
					Diagnostic::error(err)
						.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
				);
				Vec::new()
			}
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
					.with_range(err.range)
					.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
			);
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Module {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let short_module_path = self
			.module_path
			.strip_prefix(std::env::current_dir().unwrap())
			.unwrap()
			.to_str()
			.unwrap();

		let mut sorted_comments: Vec<_> = self.comments.iter().collect();
		sorted_comments.sort_by_key(|(line, _)| *line);
		let sorted_comments: std::collections::BTreeMap<_, _> = sorted_comments.into_iter().collect();

		f.debug_struct(&format!(
			"module `{}` ({})",
			self.module_name, short_module_path
		))
		.field("comments", &sorted_comments)
		.field("ast", &self.ast)
		.finish()
	}
}
