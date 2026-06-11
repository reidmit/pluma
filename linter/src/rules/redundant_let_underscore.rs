use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, PatternKind};
use compiler::{Diagnostic, Range, Reportable};

/// Flags `let _ = expr` — binding an expression to the wildcard pattern with no
/// type annotation. The `let _ =` captures nothing, so the statement is
/// equivalent to evaluating `expr` on its own line. (An annotated `let _ :: T =
/// expr` is left alone: there the annotation asserts the value's type, which
/// dropping the binding would discard.)
pub struct RedundantLetUnderscore;

impl Rule for RedundantLetUnderscore {
	fn name(&self) -> &'static str {
		"redundant-let-underscore"
	}

	fn check_expr(&self, expr: &ExprNode, _ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::Let(let_node) = &expr.kind else {
			return;
		};
		if !matches!(let_node.pattern.kind, PatternKind::Underscore) {
			return;
		}
		if let_node.type_annotation.is_some() {
			return;
		}
		// Fix: delete the `let _ = ` prefix, leaving the value expression in place.
		let prefix = Range::between(let_node.range.start, let_node.value.range.start);
		out.push(
			Finding::new(Diagnostic::report_warning(Lint).with_span(let_node.range)).with_fix(prefix, ""),
		);
	}
}

struct Lint;

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "`let _ =` binds nothing, so the `let _ =` is redundant.")
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0001"
	}

	fn help(&self) -> Option<String> {
		Some("drop the `let _ = ` and keep just the expression.".to_string())
	}
}
