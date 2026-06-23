use crate::{Context, Finding, Rule};
use compiler::ast::{CaseNode, ExprKind, ExprNode, PatternKind, PatternNode};
use compiler::{Diagnostic, Reportable};

/// Flags a `when` over a `result` whose only job is to unwrap `ok` and
/// re-propagate `err` unchanged:
///
/// ```text
/// when eval env l is ok lv {
///     <success body>
/// } is err m {
///     err m
/// }
/// ```
///
/// That whole shape is what `try` says in one line — `try lv = eval env l` binds
/// the `ok` payload and short-circuits on `err`, leaving the success body as the
/// continuation:
///
/// ```text
/// try lv = eval env l
/// <success body>
/// ```
///
/// Report-only: the rewrite swaps the operand order (`try lv = SUBJECT`),
/// re-indents the success body, and deletes the `err` arm — more than a flat span
/// edit can express without reconstructing the subject's source, so it's left to
/// the author.
///
/// Only fires on the *tail* of a block, where the `when`'s value is the block's
/// value: that's the one position where `try` (whose continuation is the rest of
/// the enclosing block) is an exact substitute. A non-tail `when` whose result is
/// bound or used can't become a `try`, so it's left alone.
pub struct WhenAsTry;

impl Rule for WhenAsTry {
	fn name(&self) -> &'static str {
		"when-as-try"
	}

	fn check_body(&self, body: &[ExprNode], _ctx: &Context, out: &mut Vec<Finding>) {
		let Some(last) = body.last() else {
			return;
		};
		let ExprKind::When(node) = &last.kind else {
			return;
		};
		// Exactly two arms: an `ok` unwrap and an `err` passthrough.
		let [a, b] = node.cases.as_slice() else {
			return;
		};
		// The two arms can be written in either order.
		let is_try =
			(is_ok_unwrap(a) && is_err_passthrough(b)) || (is_err_passthrough(a) && is_ok_unwrap(b));
		if is_try {
			out.push(Finding::new(
				Diagnostic::report_warning(Lint).with_span(last.range),
			));
		}
	}
}

/// An `is ok <pattern>` arm — the `ok` carrier with a single bound payload. The
/// payload pattern is irrefutable in any well-typed match, so it transfers
/// verbatim to `try <pattern> = …`.
fn is_ok_unwrap(case: &CaseNode) -> bool {
	matches!(
		&case.pattern.kind,
		PatternKind::Constructor(head, args) if head.variant.name == "ok" && args.len() == 1
	)
}

/// An `is err m { err m }` arm: it binds the error and immediately re-wraps the
/// *same* binding, unchanged — pure propagation, exactly what `try` does on
/// failure. A body that inspects, logs, or rewrites the error is doing real work
/// and isn't a `try`.
fn is_err_passthrough(case: &CaseNode) -> bool {
	let PatternKind::Constructor(head, args) = &case.pattern.kind else {
		return false;
	};
	if head.variant.name != "err" {
		return false;
	}
	let [
		PatternNode {
			kind: PatternKind::Identifier(bound),
			..
		},
	] = args.as_slice()
	else {
		return false;
	};
	// The arm body must be exactly `err <bound>` — re-wrapping the same name.
	let [only] = case.body.as_slice() else {
		return false;
	};
	let ExprKind::Call(call) = &only.kind else {
		return false;
	};
	let ExprKind::Identifier(callee) = &call.callee.kind else {
		return false;
	};
	if callee.name != "err" {
		return false;
	}
	matches!(
		call.args.as_slice(),
		[ExprNode { kind: ExprKind::Identifier(arg), .. }] if arg.name == bound.name
	)
}

struct Lint;

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"This `when` only unwraps `ok` and re-propagates `err` — that's what `try` does."
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0013"
	}

	fn help(&self) -> Option<String> {
		Some(
			"rewrite as `try <binding> = <subject>` followed by the success body; `try` \
			 short-circuits on `err`, so the error arm goes away."
				.to_string(),
		)
	}
}
