use crate::Rule;
use compiler::ast::{ExprKind, ExprNode, LiteralKind, Operator};
use compiler::{Diagnostic, Reportable};

/// Flags comparing an expression to a boolean literal: `x == true`, `x != false`
/// (both just `x`), and `x == false`, `x != true` (both `not x`). The comparison
/// adds nothing — the operand is already a boolean.
pub struct RedundantBoolComparison;

impl Rule for RedundantBoolComparison {
	fn name(&self) -> &'static str {
		"redundant-bool-comparison"
	}

	fn check_expr(&self, expr: &ExprNode, out: &mut Vec<Diagnostic>) {
		let ExprKind::BinaryOperation { op, left, right } = &expr.kind else {
			return;
		};
		let is_eq = match op.kind {
			Operator::Equality => true,
			Operator::Inequality => false,
			_ => return,
		};

		// Exactly one side must be the boolean literal; `true == false` is a
		// constant, not this lint's concern.
		let lit = match (bool_lit(left), bool_lit(right)) {
			(Some(_), Some(_)) | (None, None) => return,
			(Some(b), None) | (None, Some(b)) => b,
		};

		// `== true` / `!= false` leave the operand as-is; the other two negate it.
		let keep_direct = is_eq == lit;
		let help = if keep_direct {
			"drop the comparison and use the expression directly."
		} else {
			"drop the comparison and use `not <expression>` instead."
		};
		out.push(Diagnostic::report_warning(Lint(help)).with_span(expr.range));
	}
}

fn bool_lit(expr: &ExprNode) -> Option<bool> {
	match &expr.kind {
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
		write!(f, "Comparing to a boolean literal is redundant.")
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0003"
	}

	fn help(&self) -> Option<String> {
		Some(self.0.to_string())
	}
}
