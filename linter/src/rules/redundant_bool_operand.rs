use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, LiteralKind, Operator};
use compiler::{Diagnostic, Reportable};

/// Flags a boolean literal as an operand of `and` / `or`. Two shapes:
///   - identity: `x and true`, `x or false` — the literal has no effect, so the
///     expression is just `x`.
///   - constant: `x and false`, `x or true` — the expression is a constant
///     (`false` / `true`) regardless of the other operand.
pub struct RedundantBoolOperand;

impl Rule for RedundantBoolOperand {
	fn name(&self) -> &'static str {
		"redundant-bool-operand"
	}

	fn check_expr(&self, expr: &ExprNode, _ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::BinaryOperation { op, left, right } = &expr.kind else {
			return;
		};
		let is_and = match op.kind {
			Operator::LogicalAnd => true,
			Operator::LogicalOr => false,
			_ => return,
		};

		// Exactly one operand a boolean literal (`true and false` is just a
		// constant — not interesting here).
		let lit = match (bool_lit(left), bool_lit(right)) {
			(Some(_), Some(_)) | (None, None) => return,
			(Some(b), None) | (None, Some(b)) => b,
		};

		// `and true` and `or false` are identities; the other two pin the
		// whole expression to a constant.
		let identity = is_and == lit;
		let help = if identity {
			"the literal has no effect — drop it and keep the other operand.".to_string()
		} else {
			format!(
				"the whole expression is always `{}`, regardless of the other operand.",
				!is_and
			)
		};
		out.push(Finding::new(
			Diagnostic::report_warning(Lint(help)).with_span(expr.range),
		));
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

struct Lint(String);

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"Boolean literal operand makes this `and`/`or` redundant."
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0005"
	}

	fn help(&self) -> Option<String> {
		Some(self.0.clone())
	}
}
