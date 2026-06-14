use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode, TryNode};
use compiler::{Diagnostic, Reportable};
use std::collections::HashMap;

/// Modules whose members read better through a `using` block than through a
/// repeated namespace prefix: `std/css` (style rules) and `std/view` (view
/// elements). Both expose a wide flat surface that a builder typically reaches
/// for many times in one function.
const NAMESPACES: &[&str] = &["std/css", "std/view"];

/// The number of `ns.member` projections in one function that make a `using ns`
/// block worth suggesting.
const THRESHOLD: usize = 3;

/// Flags a function that projects off an imported `std/css` or `std/view`
/// namespace three or more times — `css.background`, `css.padding`, … — and
/// suggests wrapping the body in `using css { … }` so members can be written as
/// bare `.member` projections.
///
/// The count is per innermost function: occurrences inside a nested `fun` are
/// attributed to that closure (and reported on it if they reach the threshold),
/// not to the enclosing one. Occurrences already inside a matching `using` block
/// don't count — they're nothing left to wrap.
///
/// Only fires when the name actually resolves to one of those imports (via the
/// module's `use` list) and isn't shadowed by a local value, so a runtime value
/// that happens to be named `view` is never mistaken for the namespace.
pub struct PreferUsingBlock;

impl Rule for PreferUsingBlock {
	fn name(&self) -> &'static str {
		"prefer-using-block"
	}

	fn check_expr(&self, expr: &ExprNode, ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::Fun(fun) = &expr.kind else {
			return;
		};

		// This function's parameters shadow any same-named import within its body.
		// They aren't in `ctx` yet — the walker pushes them only as it descends
		// into the body, after this `check_expr` — so collect them here.
		let params: Vec<&str> = fun.params.iter().map(|p| p.ident.name.as_str()).collect();

		// Tally `ns.member` projections in this function's own body, stopping at
		// nested `fun`s and at `using` blocks already covering the namespace.
		let mut counts: HashMap<&str, usize> = HashMap::new();
		for stmt in &fun.body {
			count_projections(stmt, ctx, &params, &[], &mut counts);
		}

		// Report in a stable order so the suggestion (and its snapshot) doesn't
		// depend on hash iteration order when both namespaces qualify.
		let mut hits: Vec<(&str, usize)> = counts
			.into_iter()
			.filter(|&(_, n)| n >= THRESHOLD)
			.collect();
		hits.sort_by(|a, b| a.0.cmp(b.0));

		for (name, count) in hits {
			out.push(Finding::new(
				Diagnostic::report_warning(Lint {
					name: name.to_string(),
					count,
				})
				.with_span(fun.range),
			));
		}
	}
}

/// Recursively count `ns.member` projections rooted at an imported namespace,
/// accumulating into `counts` keyed by the local name. `params` are the enclosing
/// function's parameters, which shadow same-named imports. `suppressed` lists the
/// namespaces already inside an enclosing `using` block, whose projections don't
/// count. Does not descend into nested `fun` literals — those are tallied on
/// their own when the walker reaches them.
fn count_projections<'a>(
	expr: &'a ExprNode,
	ctx: &Context,
	params: &[&str],
	suppressed: &[&'a str],
	counts: &mut HashMap<&'a str, usize>,
) {
	match &expr.kind {
		ExprKind::FieldAccess { receiver, .. } => {
			if let ExprKind::Identifier(id) = &receiver.kind {
				let name = id.name.as_str();
				let is_namespace = ctx
					.imported_module(name)
					.is_some_and(|m| NAMESPACES.contains(&m));
				let shadowed = ctx.is_local(name) || params.contains(&name);
				if is_namespace && !shadowed && !suppressed.contains(&name) {
					*counts.entry(name).or_insert(0) += 1;
				}
			}
			count_projections(receiver, ctx, params, suppressed, counts);
		}
		ExprKind::Using { namespace, body } => {
			// Inside `using css { … }`, the css.* (and bare .*) members are already
			// what this lint would suggest — don't count them toward another wrap.
			let mut inner = suppressed.to_vec();
			inner.push(namespace.name.as_str());
			for stmt in body {
				count_projections(stmt, ctx, params, &inner, counts);
			}
		}
		// A nested function is its own counting unit — the walker visits it
		// separately, so leave its projections for that visit.
		ExprKind::Fun(_) => {}
		// Everything else: descend into sub-expressions, carrying `suppressed`.
		ExprKind::BinaryOperation { left, right, .. } => {
			count_projections(left, ctx, params, suppressed, counts);
			count_projections(right, ctx, params, suppressed, counts);
		}
		ExprKind::UnaryOperation { right, .. } => count_projections(right, ctx, params, suppressed, counts),
		ExprKind::ElementAccess { receiver, .. } => {
			count_projections(receiver, ctx, params, suppressed, counts)
		}
		ExprKind::Call(call) => {
			count_projections(&call.callee, ctx, params, suppressed, counts);
			for arg in &call.args {
				count_projections(arg, ctx, params, suppressed, counts);
			}
		}
		ExprKind::Grouping(inner) => count_projections(inner, ctx, params, suppressed, counts),
		ExprKind::Interpolation(parts) => {
			for part in parts {
				count_projections(part, ctx, params, suppressed, counts);
			}
		}
		ExprKind::Let(let_node) => count_projections(&let_node.value, ctx, params, suppressed, counts),
		ExprKind::Defer(inner) => count_projections(inner, ctx, params, suppressed, counts),
		ExprKind::Record(fields) => {
			for (_, value) in fields {
				count_projections(value, ctx, params, suppressed, counts);
			}
		}
		ExprKind::RecordUpdate { base, fields } => {
			count_projections(base, ctx, params, suppressed, counts);
			for (_, value) in fields {
				count_projections(value, ctx, params, suppressed, counts);
			}
		}
		ExprKind::Tuple(entries) => {
			for entry in entries {
				count_projections(entry, ctx, params, suppressed, counts);
			}
		}
		ExprKind::Try(TryNode { value, rest, .. }) => {
			count_projections(value, ctx, params, suppressed, counts);
			for stmt in rest {
				count_projections(stmt, ctx, params, suppressed, counts);
			}
		}
		ExprKind::List(items) => {
			for item in items {
				count_projections(item.expr(), ctx, params, suppressed, counts);
			}
		}
		ExprKind::If(if_node) => {
			count_projections(&if_node.subject, ctx, params, suppressed, counts);
			for stmt in &if_node.body {
				count_projections(stmt, ctx, params, suppressed, counts);
			}
			if let Some(else_body) = &if_node.else_body {
				for stmt in else_body {
					count_projections(stmt, ctx, params, suppressed, counts);
				}
			}
		}
		ExprKind::When(when_node) => {
			count_projections(&when_node.subject, ctx, params, suppressed, counts);
			for case in &when_node.cases {
				for stmt in &case.body {
					count_projections(stmt, ctx, params, suppressed, counts);
				}
			}
		}
		ExprKind::While(while_node) => {
			count_projections(&while_node.subject, ctx, params, suppressed, counts);
			for stmt in &while_node.body {
				count_projections(stmt, ctx, params, suppressed, counts);
			}
		}
		ExprKind::Scope(scope_node) => {
			for stmt in &scope_node.body {
				count_projections(stmt, ctx, params, suppressed, counts);
			}
		}
		// Leaves: no namespace projection to find.
		ExprKind::NamespaceAccess(_)
		| ExprKind::EmptyTuple
		| ExprKind::Identifier(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::Builtin(_)
		| ExprKind::ImplicitMember { .. } => {}
	}
}

struct Lint {
	name: String,
	count: usize,
}

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"This function projects off `{}` {} times.",
			self.name, self.count
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0009"
	}

	fn help(&self) -> Option<String> {
		Some(format!(
			"wrap the body in `using {} {{ … }}` and write members as `.member`.",
			self.name
		))
	}
}
