use crate::Rule;
use crate::eq::bodies_eq;
use compiler::ast::{ExprKind, ExprNode};
use compiler::{Diagnostic, Reportable};

/// Flags an `if` whose two branches are structurally identical:
/// `if c { E } else { E }`. The condition decides nothing — usually a sign of
/// copy-paste where one branch was meant to differ.
pub struct IdenticalBranches;

impl Rule for IdenticalBranches {
	fn name(&self) -> &'static str {
		"identical-branches"
	}

	fn check_expr(&self, expr: &ExprNode, out: &mut Vec<Diagnostic>) {
		let ExprKind::If(node) = &expr.kind else {
			return;
		};
		let Some(else_body) = &node.else_body else {
			return;
		};
		if !bodies_eq(&node.body, else_body) {
			return;
		}
		out.push(Diagnostic::report_warning(Lint).with_span(expr.range));
	}
}

struct Lint;

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Both branches of this `if` are identical.")
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0007"
	}

	fn help(&self) -> Option<String> {
		Some(
			"both arms run the same code; keep one branch (check the condition has no side effects)."
				.to_string(),
		)
	}
}
