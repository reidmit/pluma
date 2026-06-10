//! The Pluma linter: a parse-based source checker that flags stylistic and
//! correctness smells the type-checker tolerates. Like the formatter, it works
//! off the parsed AST (no analysis), so it runs on any module that parses.
//!
//! A lint is a [`Rule`]. The [`walk`] module visits every expression in a module
//! and offers it to each registered rule, so a rule only describes *what* it
//! flags — never how to traverse. Findings come back as warning [`Diagnostic`]s,
//! the same shape the compiler frontend emits, so the CLI and LSP render and
//! publish them through their existing diagnostic paths.

mod eq;
mod rules;
mod walk;

use compiler::ast::ExprNode;
use compiler::{Diagnostic, Module};
use std::path::{Path, PathBuf};

/// A single lint rule. The walker offers a rule two kinds of context as it
/// descends, in source order:
///
/// - [`check_expr`](Rule::check_expr) — one [`ExprNode`] at a time. Most rules
///   live here: they match a single expression shape (a `let`, an `if`, a
///   comparison) and flag it.
/// - [`check_body`](Rule::check_body) — a statement block (`Vec<ExprNode>`) as a
///   whole. For rules that reason about *sequences* of statements, e.g. a
///   binding immediately followed by its own use. Argument lists and tuple
///   elements are *not* bodies, so this only fires on real blocks.
///
/// Both default to no-ops, so a rule implements only the hook it needs.
pub trait Rule {
	/// A stable kebab-case identifier for the rule, e.g. `redundant-let-underscore`.
	fn name(&self) -> &'static str;

	/// Inspect one expression and push a warning diagnostic for each violation.
	fn check_expr(&self, expr: &ExprNode, out: &mut Vec<Diagnostic>) {
		let _ = (expr, out);
	}

	/// Inspect a statement block as a whole and push a warning for each violation.
	fn check_body(&self, body: &[ExprNode], out: &mut Vec<Diagnostic>) {
		let _ = (body, out);
	}
}

/// The active rule set. Adding a lint is a one-line edit here plus the rule impl.
fn rules() -> Vec<Box<dyn Rule>> {
	vec![
		Box::new(rules::RedundantLetUnderscore),
		Box::new(rules::RedundantTryUnderscore),
		Box::new(rules::RedundantBoolComparison),
		Box::new(rules::RedundantBoolOperand),
		Box::new(rules::IfReturnsBool),
		Box::new(rules::RedundantLambda),
		Box::new(rules::IdenticalBranches),
		Box::new(rules::BindThenReturn),
	]
}

/// Parse `source` as Pluma and return the lint warnings found in it. On parse
/// failure, returns the parse diagnostics as `Err` — mirrors
/// `formatter::format_source`, so callers handle "unparseable" the same way for
/// both tools. The returned warnings carry spans but no module path; use
/// [`lint_path`] when you want them anchored to a file for rendering.
pub fn lint_source(source: &[u8]) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	let mut module = Module::new("<lint>".to_string(), PathBuf::from("<lint>"));
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);

	if diagnostics.iter().any(|d| d.is_error()) {
		return Err(diagnostics);
	}

	let ast = module.ast.as_ref().expect("parser populated ast");
	let rules = rules();
	let mut out = Vec::new();
	walk::walk_module(ast, &rules, &mut out);
	Ok(out)
}

/// Like [`lint_source`], but stamps each warning with `path` so the diagnostic
/// renderer can pull the source excerpt for the caret. Used by `pluma lint`.
pub fn lint_path(path: &Path, source: &[u8]) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
	let name = path.to_string_lossy().into_owned();
	lint_source(source).map(|warnings| {
		warnings
			.into_iter()
			.map(|d| d.with_module(name.clone(), path.to_path_buf()))
			.collect()
	})
}
