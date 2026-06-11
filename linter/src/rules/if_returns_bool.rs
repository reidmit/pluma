use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, LiteralKind, PatternKind};
use compiler::{Diagnostic, Reportable};

/// Flags an `if` whose two branches each return a boolean literal:
/// `if c { true } else { false }` is just `c`, and the inverted form is `not c`.
/// Only fires on a plain boolean condition (`if c`), not a pattern match
/// (`if x is some v`), whose subject isn't usable as a boolean on its own.
pub struct IfReturnsBool;

impl Rule for IfReturnsBool {
	fn name(&self) -> &'static str {
		"if-returns-bool"
	}

	fn check_expr(&self, expr: &ExprNode, _ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::If(node) = &expr.kind else {
			return;
		};
		// Plain boolean condition only: the parser desugars `if c` to the
		// implicit pattern `is true`. A real `is <pat>` is left alone.
		if !matches!(
			&node.pattern.kind,
			PatternKind::Literal(lit) if matches!(lit.kind, LiteralKind::Bool(true))
		) {
			return;
		}

		let Some(else_body) = &node.else_body else {
			return;
		};
		let (Some(then_val), Some(else_val)) = (single_bool(&node.body), single_bool(else_body)) else {
			return;
		};
		// Equal branches (`{ true } else { true }`) are the identical-branches
		// lint's job, not this one.
		if then_val == else_val {
			return;
		}

		let help = if then_val {
			"replace the whole `if` with its condition."
		} else {
			"replace the whole `if` with the negated condition (`!<condition>`)."
		};
		// Report-only: no autofix. Deleting the `if … {` / `} else { … }` around
		// the condition is unsafe in `else if` position (the `if` is the entire
		// `else` arm, and `else <bare-expr>` doesn't parse), and these often want
		// a human rewrite (e.g. an `or`-chain of byte tests) rather than a bare
		// condition. Fixing it by hand is the right call.
		out.push(Finding::new(
			Diagnostic::report_warning(Lint(help)).with_span(expr.range),
		));
	}
}

/// `Some(b)` when `body` is exactly one boolean-literal expression.
fn single_bool(body: &[ExprNode]) -> Option<bool> {
	let [only] = body else {
		return None;
	};
	match &only.kind {
		ExprKind::Literal(lit) => match lit.kind {
			LiteralKind::Bool(b) => Some(b),
			_ => None,
		},
		_ => None,
	}
}

struct Lint(&'static str);

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"This `if` just returns a boolean — use the condition itself."
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0004"
	}

	fn help(&self) -> Option<String> {
		Some(self.0.to_string())
	}
}
