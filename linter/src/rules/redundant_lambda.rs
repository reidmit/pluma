use crate::{Context, Finding, Rule};
use compiler::ast::{ExprKind, ExprNode};
use compiler::{Diagnostic, Reportable};

/// Flags a function literal that only forwards its parameters, unchanged and in
/// order, to another call: `fun x { f x }`, `fun x y { f x y }`. Since Pluma's
/// calls are uncurried, the wrapper is exactly the callee — `f` — so it can be
/// dropped.
///
/// One exception, skipped entirely (not even reported): a callee that *projects a
/// field off a local value*, `fun t { s.spawn t }` where `s` is a parameter /
/// handle / `let`. That wrapper is load-bearing, not redundant — `s.spawn` may be
/// a scope-handle method (call-only syntax the analyzer rewrites) or a
/// row-polymorphic projection whose type the wrapper preserves. A projection off
/// a *namespace* (`color.named`, `math.combine`), where the root is not a local,
/// is fine and gets the autofix like a bare identifier.
pub struct RedundantLambda;

impl Rule for RedundantLambda {
	fn name(&self) -> &'static str {
		"redundant-lambda"
	}

	fn check_expr(&self, expr: &ExprNode, ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::Fun(fun) = &expr.kind else {
			return;
		};
		// A zero-arg `fun { f () }` is a thunk, not an eta-wrapper — leave it.
		if fun.params.is_empty() {
			return;
		}
		let [body] = fun.body.as_slice() else {
			return;
		};
		let ExprKind::Call(call) = &body.kind else {
			return;
		};
		if call.args.len() != fun.params.len() {
			return;
		}
		// Each argument must be exactly the matching parameter, in order.
		let forwards = fun.params.iter().zip(&call.args).all(
			|(param, arg)| matches!(&arg.kind, ExprKind::Identifier(id) if id.name == param.ident.name),
		);
		if !forwards {
			return;
		}
		// The callee must not depend on a parameter — otherwise it isn't a plain
		// forward (e.g. `fun x { (g x) x }`).
		let params: Vec<&str> = fun.params.iter().map(|p| p.ident.name.as_str()).collect();
		if mentions(&call.callee, &params) {
			return;
		}
		// Skip a projection off a local value (`s.spawn`): load-bearing, not
		// redundant. A projection off a namespace (`color.named`) has a non-local
		// root and is fine.
		if let Some(root) = projection_root(&call.callee) {
			if ctx.is_local(root) {
				return;
			}
		}

		let help = match callee_text(&call.callee) {
			Some(name) => format!("replace the wrapper with `{}` directly.", name),
			None => "replace the wrapper with the function it forwards to.".to_string(),
		};
		let mut finding = Finding::new(Diagnostic::report_warning(Lint(help)).with_span(fun.range));
		// Fix: replace the whole `fun … { callee … }` with the callee reference.
		if let Some(name) = callee_text(&call.callee) {
			finding = finding.with_fix(fun.range, name);
		}
		out.push(finding);
	}
}

/// For a callee that projects a field/element (`a.b`, `a.b.c`, `a.0`), the name
/// of the base it projects from (`a`). `None` for a plain identifier or anything
/// that isn't a projection rooted at an identifier.
fn projection_root(expr: &ExprNode) -> Option<&str> {
	match &expr.kind {
		ExprKind::FieldAccess { receiver, .. } | ExprKind::ElementAccess { receiver, .. } => {
			base_ident(receiver)
		}
		_ => None,
	}
}

fn base_ident(expr: &ExprNode) -> Option<&str> {
	match &expr.kind {
		ExprKind::Identifier(id) => Some(&id.name),
		ExprKind::FieldAccess { receiver, .. } | ExprKind::ElementAccess { receiver, .. } => {
			base_ident(receiver)
		}
		ExprKind::Grouping(inner) => base_ident(inner),
		_ => None,
	}
}

/// The source text of a plain reference callee, reconstructed from the AST:
/// `f`, `math.add` (a parse-time `FieldAccess` chain or analyzed
/// `NamespaceAccess`), `tuple.0`. `None` for anything that isn't a simple
/// reference path (so no autofix is offered, only the report).
fn callee_text(expr: &ExprNode) -> Option<String> {
	match &expr.kind {
		ExprKind::Identifier(id) => Some(id.name.clone()),
		ExprKind::NamespaceAccess(path) => Some(
			path
				.iter()
				.map(|p| p.name.as_str())
				.collect::<Vec<_>>()
				.join("."),
		),
		ExprKind::FieldAccess { receiver, field } => {
			Some(format!("{}.{}", callee_text(receiver)?, field.name))
		}
		ExprKind::ElementAccess { receiver, index } => {
			Some(format!("{}.{}", callee_text(receiver)?, index))
		}
		_ => None,
	}
}

/// Whether `expr` references any of `names`. Conservative: shapes it doesn't
/// descend into return `true` (assume a reference), so the lint only fires when
/// the callee is provably parameter-free.
fn mentions(expr: &ExprNode, names: &[&str]) -> bool {
	match &expr.kind {
		ExprKind::Identifier(id) => names.contains(&id.name.as_str()),
		// A namespace path (`module.value`) names no locals.
		ExprKind::NamespaceAccess(_) | ExprKind::Literal(_) | ExprKind::EmptyTuple => false,
		ExprKind::FieldAccess { receiver, .. } | ExprKind::ElementAccess { receiver, .. } => {
			mentions(receiver, names)
		}
		ExprKind::Grouping(inner) => mentions(inner, names),
		ExprKind::Call(call) => {
			mentions(&call.callee, names) || call.args.iter().any(|a| mentions(a, names))
		}
		// Unmodeled shapes (control flow, nested `fun`, etc.): assume a reference.
		_ => true,
	}
}

struct Lint(String);

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"This function only forwards its arguments to another call."
		)
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0006"
	}

	fn help(&self) -> Option<String> {
		Some(self.0.clone())
	}
}
