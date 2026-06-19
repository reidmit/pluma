use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode};
use compiler::{Diagnostic, Reportable};

/// Inside a `using ns { … }` block, every member of `ns` is already reachable as
/// a bare `.member` projection, so spelling one `ns.member` repeats the namespace
/// the block just established. This flags the redundant prefix and offers to drop
/// it — `using css { css.padding … }` becomes `using css { .padding … }`.
///
/// Fires only when a single `using` block is in scope. With two or more nested
/// `using` blocks a bare `.member` resolves in the *innermost* namespace, so an
/// explicit `ns.` prefix may be deliberately naming which block a member comes
/// from — that disambiguation is left to the author.
///
/// Skipped when the name is shadowed by a local value: `css.width` where `css` is
/// a `let`-bound record is an ordinary field access, not a namespace projection,
/// and dropping the prefix would change what it means.
pub struct RedundantUsingPrefix;

impl Rule for RedundantUsingPrefix {
	fn name(&self) -> &'static str {
		"redundant-using-prefix"
	}

	fn check_expr(&self, expr: &ExprNode, ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::FieldAccess { receiver, field } = &expr.kind else {
			return;
		};
		let ExprKind::Identifier(id) = &receiver.kind else {
			return;
		};
		let name = id.name.as_str();

		// Exactly one `using` block in scope, and it names this projection's
		// namespace. Two or more blocks mean a bare `.member` would resolve in the
		// innermost one, so the prefix may be disambiguating — stay quiet.
		let enclosing = ctx.enclosing_using();
		if enclosing.len() != 1 || enclosing[0] != name {
			return;
		}

		// A local of the same name shadows the namespace, making this a genuine
		// field access; dropping the prefix would change what it refers to.
		if ctx.is_local(name) {
			return;
		}

		let finding = Finding::new(
			Diagnostic::report_warning(Lint {
				namespace: name.to_string(),
				member: field.name.clone(),
			})
			.with_span(receiver.range),
		)
		// Drop the `<name>` before the dot; the leading `.member` left behind
		// resolves in the enclosing `using` namespace.
		.with_fix(receiver.range, "");
		out.push(finding);
	}
}

struct Lint {
	namespace: String,
	member: String,
}

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"Redundant `{}` prefix inside `using {}`.",
			self.namespace, self.namespace
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0010"
	}

	fn help(&self) -> Option<String> {
		Some(format!(
			"drop the `{}` prefix and write the member as `.{}`.",
			self.namespace, self.member
		))
	}
}
