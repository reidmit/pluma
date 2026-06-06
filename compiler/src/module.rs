use crate::Token;
use crate::ast::*;
use crate::diagnostic::*;
use crate::parser::*;
use crate::tokenizer::*;
use crate::types::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// What a module exposes to anyone that `use`s it. Populated by the Analyzer
// after type inference.
//
// `values` are top-level value defs (including alias constructor functions).
// `aliases` are alias type defs (name -> the resolved underlying type).
// `enums` are enum type defs (name -> exported enum signature).
//
// Types inside this struct carry the defining module's bare enum names (e.g.
// `Type::Enum("color", _)`); the importing analyzer qualifies them with the
// defining module's fully-qualified name when pulling them in.
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone, Default)]
pub struct ModuleExports {
	pub values: HashMap<String, Type>,
	pub aliases: HashMap<String, Type>,
	pub enums: HashMap<String, EnumExport>,
	// Class constraints attached to constrained values. The `dispatch_var`
	// shares its tyvar ids with the corresponding entry in `values`; on
	// import both are freshened together so the constraint over the value
	// flows into the importing module's constraint set.
	pub value_constraints: HashMap<String, Vec<ValueConstraintExport>>,
	// Typeclass instances declared in this module. Importers seed these
	// into their own analyzer at init so they can discharge constraints
	// against them. Param-tyvar ids inside `head_type` and `where_clauses`
	// are canonical (0..param_count-1); the importer mints fresh ids
	// before inserting into its local registry.
	pub instances: Vec<InstanceExport>,
	// Public traits declared in this module. Importers register these into
	// their own trait pool so the methods dispatch by bare name (Rust-style
	// `use Trait`) and are reachable as `module.trait.method`. The trait's
	// param tyvar is canonical (`Var(0)`); the importer mints a fresh id.
	pub traits: HashMap<String, TraitExport>,
	// Names of top-level defs that exist in this module but aren't visible
	// to importers (no `public`/`opaque` keyword). Carried so importers can
	// report a precise "`x` is private to module `y`" diagnostic instead of
	// a bare "not found". Opaque enum *type* names are NOT listed here (the
	// type is accessible — only its constructors are withheld).
	pub private: std::collections::HashSet<String>,
}

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct ValueConstraintExport {
	pub trait_name: String,
	pub dispatch_var: Type,
}

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct InstanceExport {
	pub trait_name: String,
	// Concrete `Type` for concrete instances; for parametric instances,
	// contains `Type::Var(i)` placeholders for `i in 0..param_count`.
	pub head_type: Type,
	pub param_count: usize,
	// `(trait_name, canonical_var_idx)` for each `where`-clause constraint.
	pub where_clauses: Vec<(String, usize)>,
	pub instance_slot_name: String,
}

// A trait's signature, exported across module boundaries. The trait's
// single param tyvar is referenced as `Type::Var(0)` in `method_types`;
// the importing analyzer mints a fresh local var and substitutes it.
// `defaults` carries the AST template for each method that has a default
// body, so an instance in the importing module that omits a defaulted
// method can clone it (exports are in-memory, so this is a plain clone).
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct TraitExport {
	pub method_order: Vec<String>,
	pub method_types: HashMap<String, Type>,
	pub defaults: HashMap<String, ExprNode>,
}

// A generic enum's signature, exported across module boundaries. Variant
// params reference type vars by *canonical* ids `0..param_count-1`. The
// importing analyzer mints fresh local vars and substitutes the canonical
// ids before storing into its own `enum_defs`.
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct EnumExport {
	pub param_count: usize,
	pub variants: Vec<(String, Vec<Type>)>,
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
				Diagnostic::error(format!(
					"Could not read module `{}`: {}",
					self.module_name, err
				))
				.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
			),
		}
	}

	// Parse directly from in-memory source bytes. Used for the prelude
	// module (baked into the compiler binary) and any other synthetic
	// module that doesn't live on disk.
	pub fn parse_from_bytes(&mut self, bytes: Vec<u8>, diagnostics: &mut Vec<Diagnostic>) {
		self.build_ast(bytes, diagnostics);
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
				Diagnostic::report(err)
					.with_range(err.range)
					.with_module(self.module_name.clone(), self.module_path.to_path_buf()),
			);
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Module {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// Trim the cwd off for readability when possible; fall back to the
		// full path if the module isn't under cwd (e.g. when integration
		// tests run from a different working directory).
		let cwd = std::env::current_dir().ok();
		let short_module_path = cwd
			.as_ref()
			.and_then(|cwd| self.module_path.strip_prefix(cwd).ok())
			.unwrap_or(&self.module_path)
			.to_str()
			.unwrap_or("<invalid utf-8 path>");

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
