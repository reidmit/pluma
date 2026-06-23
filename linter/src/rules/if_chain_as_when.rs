use crate::eq::expr_eq;
use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, IfNode, LiteralKind, PatternKind};
use compiler::{Diagnostic, Reportable};

/// Flags an `if` / `else if` chain that dispatches on a single value's shape:
/// every link tests the *same* subject against a pattern
/// (`if x is some v { … } else if x is none { … }`, or a run of literal cases).
/// That's what `when` is for — and unlike the chain, a `when` is
/// exhaustiveness-checked, so a forgotten variant becomes a compile error
/// instead of silently falling through the chain's tail.
///
/// Report-only: the rewrite drops the repeated subject from every arm and may
/// newly require a catch-all (for open types like `int`), so it's left to the
/// author. A plain boolean chain (`if a { … } else if b { … }`) is untouched —
/// its links test *different* subjects, so it isn't a single-value dispatch.
pub struct IfChainAsWhen;

impl Rule for IfChainAsWhen {
	fn name(&self) -> &'static str {
		"if-chain-as-when"
	}

	fn check_expr(&self, expr: &ExprNode, ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::If(node) = &expr.kind else {
			return;
		};
		// Only reason about a chain from its head; an `else if` continuation is
		// part of the chain its head already reported on.
		if ctx.is_else_if_link() {
			return;
		}

		// Walk the chain, following each `else if` (an else-body holding a single
		// `if`), collecting every link's subject and pattern.
		let mut subjects: Vec<&ExprNode> = Vec::new();
		let mut patterns: Vec<&PatternKind> = Vec::new();
		let mut link: &IfNode = node;
		loop {
			subjects.push(&link.subject);
			patterns.push(&link.pattern.kind);
			match link.else_body.as_deref() {
				Some([cont]) => match &cont.kind {
					ExprKind::If(next) => link = next,
					_ => break,
				},
				_ => break,
			}
		}

		// A single link is an if-let, not a dispatch — leave it as an `if`.
		if subjects.len() < 2 {
			return;
		}

		// Every link must test the same subject against a real dispatch pattern.
		let head = subjects[0];
		let dispatches_one_value =
			subjects.iter().all(|s| expr_eq(s, head)) && patterns.iter().all(|p| is_dispatch_pattern(p));
		if dispatches_one_value {
			out.push(Finding::new(
				Diagnostic::report_warning(Lint).with_span(expr.range),
			));
		}
	}
}

/// A pattern that classifies a value's shape — a constructor, a tuple/record/list
/// shape, an interpolation, or a non-boolean literal. Excludes `_`/bindings
/// (catch-alls, not dispatch arms) and boolean literals: a two-way boolean split
/// belongs as an `if`, not a `when` (see the when-as-if lint).
fn is_dispatch_pattern(kind: &PatternKind) -> bool {
	match kind {
		PatternKind::Constructor(..)
		| PatternKind::Tuple(_)
		| PatternKind::Record { .. }
		| PatternKind::List { .. }
		| PatternKind::Interpolation(_) => true,
		PatternKind::Literal(lit) => !matches!(lit.kind, LiteralKind::Bool(_)),
		PatternKind::Identifier(_) | PatternKind::Underscore => false,
	}
}

struct Lint;

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"This `if`/`else if` chain dispatches on one value — a `when` would be checked for exhaustiveness."
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0012"
	}

	fn help(&self) -> Option<String> {
		Some(
			"rewrite as `when <subject> is <pattern> { … } …` so the compiler verifies every case \
			 is handled (open types like `int` still need an `else`)."
				.to_string(),
		)
	}
}
