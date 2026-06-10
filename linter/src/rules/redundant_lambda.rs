use crate::Rule;
use compiler::ast::{ExprKind, ExprNode};
use compiler::{Diagnostic, Reportable};

/// Flags a function literal that only forwards its parameters, unchanged and in
/// order, to another call: `fun x { f x }`, `fun x y { f x y }`. Since Pluma's
/// calls are uncurried, the wrapper is exactly the callee — `f` — so it can be
/// dropped. Only fires when the callee doesn't itself mention a parameter (so
/// `fun x { x add 1 }`-style isn't touched).
pub struct RedundantLambda;

impl Rule for RedundantLambda {
	fn name(&self) -> &'static str {
		"redundant-lambda"
	}

	fn check_expr(&self, expr: &ExprNode, out: &mut Vec<Diagnostic>) {
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

		let help = match callee_name(&call.callee) {
			Some(name) => format!("replace the wrapper with `{}` directly.", name),
			None => "replace the wrapper with the function it forwards to.".to_string(),
		};
		out.push(Diagnostic::report_warning(Lint(help)).with_span(fun.range));
	}
}

/// A display name for a simple callee (`f`, `math.add`); `None` for anything
/// that isn't a plain identifier or namespace path.
fn callee_name(expr: &ExprNode) -> Option<String> {
	match &expr.kind {
		ExprKind::Identifier(id) => Some(id.name.clone()),
		ExprKind::NamespaceAccess(path) => Some(
			path
				.iter()
				.map(|p| p.name.as_str())
				.collect::<Vec<_>>()
				.join("."),
		),
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
