use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, PatternKind};
use compiler::{Diagnostic, Reportable};

/// Flags a block that ends `let x = E` immediately followed by `x` — the binding
/// is created only to be returned, so the block can end with `E` directly.
/// Skips an annotated `let x :: T = E`, where dropping the `let` would discard
/// the type assertion.
pub struct BindThenReturn;

impl Rule for BindThenReturn {
	fn name(&self) -> &'static str {
		"bind-then-return"
	}

	fn check_body(&self, body: &[ExprNode], _ctx: &Context, out: &mut Vec<Finding>) {
		let [.., second_last, last] = body else {
			return;
		};
		// The block must end with a bare identifier...
		let ExprKind::Identifier(returned) = &last.kind else {
			return;
		};
		// ...immediately preceded by a `let` binding that same name.
		let ExprKind::Let(let_node) = &second_last.kind else {
			return;
		};
		if let_node.type_annotation.is_some() {
			return;
		}
		let PatternKind::Identifier(bound) = &let_node.pattern.kind else {
			return;
		};
		if bound.name != returned.name {
			return;
		}
		out.push(Finding::new(
			Diagnostic::report_warning(Lint).with_span(let_node.range),
		));
	}
}

struct Lint;

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"This binding is returned immediately, so the `let` is unnecessary."
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0008"
	}

	fn help(&self) -> Option<String> {
		Some("drop the `let` and end the block with the expression directly.".to_string())
	}
}
