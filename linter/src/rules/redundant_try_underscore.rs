use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, PatternKind};
use compiler::{Diagnostic, Range, Reportable};

/// Flags `try _ = expr` — propagating a failure but binding the success value to
/// the wildcard pattern. The `_ =` captures nothing, and `try expr` is the exact
/// bindingless sugar for it, so the `_ =` is redundant. (Only the explicit
/// `binding` form is flagged; `try expr` already parses bindingless.)
pub struct RedundantTryUnderscore;

impl Rule for RedundantTryUnderscore {
	fn name(&self) -> &'static str {
		"redundant-try-underscore"
	}

	fn check_expr(&self, expr: &ExprNode, _ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::Try(try_node) = &expr.kind else {
			return;
		};
		// `binding = false` is already the bindingless `try expr` surface.
		if !try_node.binding {
			return;
		}
		if !matches!(try_node.pattern.kind, PatternKind::Underscore) {
			return;
		}
		// The node's own range runs through the rest of the block; point the
		// caret at just the `try _ = <value>` head instead.
		let span = Range::between(try_node.range.start, try_node.value.range.end);
		// Fix: replace `try _ = ` with `try `, leaving the value in place.
		let head = Range::between(try_node.range.start, try_node.value.range.start);
		out.push(Finding::new(Diagnostic::report_warning(Lint).with_span(span)).with_fix(head, "try "));
	}
}

struct Lint;

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "`try _ =` binds nothing, so the `_ =` is redundant.")
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0002"
	}

	fn help(&self) -> Option<String> {
		Some("drop the `_ = ` and write just `try <expression>`.".to_string())
	}
}
