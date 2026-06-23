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
use compiler::{Diagnostic, Module, Point, Range};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An autofix: replace the source spanning `range` with `replacement`. A rule
/// attaches one to a [`Finding`] when the violation has a mechanical rewrite
/// (e.g. dropping a `let _ =` prefix). `pluma lint --fix` applies them.
#[derive(Clone)]
pub struct Fix {
	pub range: Range,
	pub replacement: String,
}

/// One lint result: the [`Diagnostic`] to report, plus any autofixes. A finding
/// usually has zero or one fix; a few rules need two disjoint edits (e.g.
/// deleting the `if … {` and `} else { … }` around a kept subject).
pub struct Finding {
	pub diagnostic: Diagnostic,
	pub fixes: Vec<Fix>,
}

impl Finding {
	/// A finding with no autofix.
	pub fn new(diagnostic: Diagnostic) -> Self {
		Finding {
			diagnostic,
			fixes: Vec::new(),
		}
	}

	/// Attach an edit that replaces `range` with `replacement`. Chainable for a
	/// fix that needs more than one edit.
	pub fn with_fix(mut self, range: Range, replacement: impl Into<String>) -> Self {
		self.fixes.push(Fix {
			range,
			replacement: replacement.into(),
		});
		self
	}
}

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
/// The lexical context a rule sees at a node: which names are in scope as *local
/// values* (function parameters, `scope as` handles, `let` bindings). Lets a rule
/// tell a projection on a runtime value (`s.spawn`, where `s` is a local) from
/// one on a module or type namespace (`color.named`) — they're the same
/// `FieldAccess` shape in the parsed AST.
pub struct Context {
	frames: Vec<Vec<String>>,
	/// Local import name → fully-qualified module name, e.g. `css` → `std/css`
	/// (or the alias for `use std/css as c`). Module-wide, set once before the
	/// walk; lets a rule recognize that a `css.rule` projection lands on an
	/// imported module rather than an unrelated local of the same name.
	imports: HashMap<String, String>,
	/// Local names of the enclosing `using` blocks (innermost last), pushed as
	/// the walk descends into a `using` body and popped on the way out. Lets a
	/// rule see that a projection — or a nested `fun` it's about to flag — already
	/// sits inside a `using` block, even across the function boundary the walker
	/// crosses to visit that closure as its own unit.
	using: Vec<String>,
	/// Whether the node currently being offered to rules is an `else if`
	/// continuation — the inner `if` of an enclosing `if`'s else-body. Set by the
	/// walker just before that node's rule offer and cleared right after, so it
	/// scopes to exactly one node. Lets a chain-reasoning rule fire only at the
	/// chain head instead of once per link.
	else_if_link: bool,
}

impl Context {
	fn new() -> Self {
		Context {
			frames: Vec::new(),
			imports: HashMap::new(),
			using: Vec::new(),
			else_if_link: false,
		}
	}

	/// Record the module's `use` bindings. Called once by the walker before
	/// descending, so every rule sees the same import set regardless of position.
	pub(crate) fn set_imports(&mut self, imports: HashMap<String, String>) {
		self.imports = imports;
	}

	/// The fully-qualified module a local name was imported as (`std/css`), or
	/// `None` if the name isn't an import. A name that is also shadowed by a local
	/// value is still reported here — callers that care pair this with
	/// [`is_local`](Self::is_local).
	pub fn imported_module(&self, name: &str) -> Option<&str> {
		self.imports.get(name).map(String::as_str)
	}

	fn push(&mut self, names: Vec<String>) {
		self.frames.push(names);
	}

	fn pop(&mut self) {
		self.frames.pop();
	}

	/// Bind another name in the current (innermost) frame — used for `let`
	/// statements, which come into scope mid-block.
	fn bind(&mut self, name: String) {
		if let Some(frame) = self.frames.last_mut() {
			frame.push(name);
		}
	}

	/// Whether `name` is bound as a local value anywhere in the enclosing scopes.
	pub fn is_local(&self, name: &str) -> bool {
		self.frames.iter().flatten().any(|n| n == name)
	}

	/// Enter a `using <name>` block; `name` is the namespace's local import name.
	fn enter_using(&mut self, name: String) {
		self.using.push(name);
	}

	/// Leave the innermost `using` block.
	fn leave_using(&mut self) {
		self.using.pop();
	}

	/// The local names of the `using` blocks currently in scope (innermost last).
	pub fn enclosing_using(&self) -> &[String] {
		&self.using
	}

	/// Mark the next node offered to rules as an `else if` continuation.
	fn mark_else_if_link(&mut self) {
		self.else_if_link = true;
	}

	/// Clear the else-if-continuation mark; called after each node's rule offer so
	/// it never leaks into that node's sub-expressions.
	fn clear_else_if_link(&mut self) {
		self.else_if_link = false;
	}

	/// Whether the current node is an `else if` continuation of an enclosing `if`.
	pub fn is_else_if_link(&self) -> bool {
		self.else_if_link
	}
}

/// Both default to no-ops, so a rule implements only the hook it needs.
pub trait Rule {
	/// A stable kebab-case identifier for the rule, e.g. `redundant-let-underscore`.
	fn name(&self) -> &'static str;

	/// Inspect one expression and push a [`Finding`] for each violation. `ctx`
	/// carries the local-value bindings in scope at this node.
	fn check_expr(&self, expr: &ExprNode, ctx: &Context, out: &mut Vec<Finding>) {
		let _ = (expr, ctx, out);
	}

	/// Inspect a statement block as a whole and push a [`Finding`] for each violation.
	fn check_body(&self, body: &[ExprNode], ctx: &Context, out: &mut Vec<Finding>) {
		let _ = (body, ctx, out);
	}

	/// Inspect a top-level definition's value expression as a single unit. Unlike
	/// [`check_expr`](Rule::check_expr), which fires on every node, this fires once
	/// per value definition (`def name = …`), so a rule can reason about the
	/// definition as a whole — e.g. suggesting a `using` block around a non-function
	/// value whose body repeatedly projects off one namespace.
	fn check_definition(&self, value: &ExprNode, ctx: &Context, out: &mut Vec<Finding>) {
		let _ = (value, ctx, out);
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
		Box::new(rules::PreferUsingBlock),
		Box::new(rules::RedundantUsingPrefix),
		Box::new(rules::RedundantLambda),
		Box::new(rules::IdenticalBranches),
		Box::new(rules::BindThenReturn),
		Box::new(rules::WhenAsIf),
		Box::new(rules::IfChainAsWhen),
		Box::new(rules::WhenAsTry),
	]
}

/// Parse `source` and collect every [`Finding`]. On parse failure, returns the
/// parse diagnostics as `Err` — mirrors `formatter::format_source`.
fn collect(source: &[u8]) -> Result<Vec<Finding>, Vec<Diagnostic>> {
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	let mut module = Module::new("<lint>".to_string(), PathBuf::from("<lint>"));
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);

	if diagnostics.iter().any(|d| d.is_error()) {
		return Err(diagnostics);
	}

	let ast = module.ast.as_ref().expect("parser populated ast");
	let rules = rules();
	let mut ctx = Context::new();
	let mut out = Vec::new();
	walk::walk_module(ast, &rules, &mut ctx, &mut out);
	Ok(out)
}

/// Parse `source` as Pluma and return the lint warnings found in it. On parse
/// failure, returns the parse diagnostics as `Err`. The returned warnings carry
/// spans but no module path; use [`lint_path`] when you want them anchored to a
/// file for rendering.
pub fn lint_source(source: &[u8]) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
	collect(source).map(|findings| findings.into_iter().map(|f| f.diagnostic).collect())
}

/// Like [`lint_source`], but keeps each warning's autofix edits so a caller can
/// offer them individually — e.g. the language server turning one finding into a
/// single editor quick-fix. On parse failure, returns the parse diagnostics as
/// `Err`, mirroring [`lint_source`]. Findings carry spans but no module path.
pub fn lint_findings(source: &[u8]) -> Result<Vec<Finding>, Vec<Diagnostic>> {
	collect(source)
}

/// Apply every available autofix to `source`, returning the rewritten text — or
/// `None` when nothing changed. Fixes are applied right-to-left so earlier edits
/// don't shift later spans; any fix overlapping an already-applied one is
/// skipped. The result is *not* reformatted — run the formatter afterward to
/// canonicalize whitespace the rewrites may have left behind.
pub fn fix_source(source: &[u8]) -> Result<Option<String>, Vec<Diagnostic>> {
	let fixes: Vec<Fix> = collect(source)?.into_iter().flat_map(|f| f.fixes).collect();
	if fixes.is_empty() {
		return Ok(None);
	}

	let line_starts = line_starts(source);
	let mut spans: Vec<(usize, usize, String)> = fixes
		.into_iter()
		.filter_map(|fix| {
			let start = offset(&line_starts, fix.range.start)?;
			let end = offset(&line_starts, fix.range.end)?;
			(start <= end && end <= source.len()).then_some((start, end, fix.replacement))
		})
		.collect();
	spans.sort_by_key(|(start, ..)| *start);

	let mut result = source.to_vec();
	let mut next_start = usize::MAX;
	for (start, end, replacement) in spans.into_iter().rev() {
		// Skip a fix that overlaps one already applied to its right.
		if end > next_start {
			continue;
		}
		result.splice(start..end, replacement.into_bytes());
		next_start = start;
	}

	match String::from_utf8(result) {
		Ok(text) => Ok(Some(text)),
		Err(_) => Err(vec![Diagnostic::error("autofix produced invalid UTF-8")]),
	}
}

/// Byte offset of the start of each (0-based) source line.
fn line_starts(source: &[u8]) -> Vec<usize> {
	let mut starts = vec![0usize];
	for (i, &b) in source.iter().enumerate() {
		if b == b'\n' {
			starts.push(i + 1);
		}
	}
	starts
}

/// Absolute byte offset of a `Point`. Columns are byte offsets within their line
/// (the tokenizer's convention), so this is exact for non-ASCII source too.
fn offset(line_starts: &[usize], point: Point) -> Option<usize> {
	line_starts.get(point.line).map(|start| start + point.col)
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
