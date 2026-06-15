use compiler::Module;
use compiler::ast::*;
use compiler::types::Type;

// A single inferred-type hint: render `label` immediately after the binder at
// (`line`, `col`). Built eagerly at analysis time from the typed AST — the
// same Send-only `(position, String)` shape the hover index uses, so it caches
// across the publish await without holding the non-`Send` `Module`.
#[derive(Clone)]
pub struct InlayHint {
	pub line: u32,
	pub col: u32,
	pub label: String,
}

// Inlay type hints for a module: one per un-annotated `let`/`try` binding and
// per lambda parameter, showing the type the analyzer inferred. This makes
// Pluma's near-total type inference *visible* without forcing annotations.
pub fn build_hints(module: &Module) -> Vec<InlayHint> {
	let mut hints = Vec::new();
	if let Some(ast) = module.ast.as_ref() {
		for def in &ast.body {
			walk_def(def, &mut hints);
		}
	}
	hints
}

// A binder's type rendered as ` : T`, or `None` when there's nothing useful to
// show. We suppress two cases that would only add noise:
//   * `Unknown` — analysis couldn't resolve a type (e.g. an upstream error).
//   * a bare type variable (`Var`) — an unconstrained generic like `a`; the
//     hint `: a` tells the reader nothing. Partially-generic types such as
//     `list a` still resolve to a concrete head, so those are kept.
// Long types are truncated so a single hint never blows out the line.
fn render(ty: &Type) -> Option<String> {
	if matches!(ty, Type::Unknown | Type::Var(_)) {
		return None;
	}
	let mut text = format!(": {}", ty);
	const MAX: usize = 48;
	if text.chars().count() > MAX {
		text = text.chars().take(MAX - 1).collect::<String>();
		text.push('…');
	}
	Some(text)
}

fn push(hints: &mut Vec<InlayHint>, after: &compiler::Range, ty: &Type) {
	if let Some(label) = render(ty) {
		hints.push(InlayHint {
			line: after.end.line as u32,
			col: after.end.col as u32,
			label,
		});
	}
}

fn walk_def(def: &DefinitionNode, hints: &mut Vec<InlayHint>) {
	match &def.kind {
		DefinitionKind::Expr(expr) => walk_expr(expr, hints),
		DefinitionKind::Instance(inst) => {
			for method in &inst.methods {
				walk_def(method, hints);
			}
		}
		DefinitionKind::Trait(t) => {
			for m in &t.methods {
				if let Some(default) = &m.default {
					walk_expr(default, hints);
				}
			}
		}
		DefinitionKind::Alias(_) | DefinitionKind::Enum(_) => {}
	}
}

fn walk_fun(f: &FunNode, hints: &mut Vec<InlayHint>) {
	// Lambda params are never annotated inline in Pluma's surface (`fun x { }`),
	// so every param's type is inferred — always a hint candidate.
	for p in &f.params {
		push(hints, &p.ident.range, &p.ty);
	}
	for e in &f.body {
		walk_expr(e, hints);
	}
}

fn walk_expr(expr: &ExprNode, hints: &mut Vec<InlayHint>) {
	match &expr.kind {
		ExprKind::Identifier(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::EmptyTuple
		| ExprKind::Builtin(_)
		| ExprKind::ImplicitMember { .. }
		| ExprKind::NamespaceAccess(_) => {}
		ExprKind::BinaryOperation { left, right, .. } => {
			walk_expr(left, hints);
			walk_expr(right, hints);
		}
		ExprKind::UnaryOperation { right, .. } => walk_expr(right, hints),
		ExprKind::ElementAccess { receiver, .. } | ExprKind::FieldAccess { receiver, .. } => {
			walk_expr(receiver, hints)
		}
		ExprKind::Fun(f) => walk_fun(f, hints),
		ExprKind::Call(c) => {
			walk_expr(&c.callee, hints);
			for arg in &c.args {
				walk_expr(arg, hints);
			}
		}
		ExprKind::Grouping(inner) | ExprKind::Defer(inner) => walk_expr(inner, hints),
		ExprKind::Interpolation(parts) | ExprKind::Tuple(parts) => {
			for p in parts {
				walk_expr(p, hints);
			}
		}
		ExprKind::List(items) => {
			for item in items {
				walk_expr(item.expr(), hints);
			}
		}
		ExprKind::Let(l) => {
			// Only un-annotated identifier bindings get a hint: an explicit
			// `let x :: T = …` already shows the type, and destructuring
			// patterns have no single binder to anchor one hint on.
			if l.type_annotation.is_none() {
				if let PatternKind::Identifier(id) = &l.pattern.kind {
					push(hints, &id.range, &l.value.ty);
				}
			}
			walk_expr(&l.value, hints);
		}
		ExprKind::Record(fields) => {
			for (_, value) in fields {
				walk_expr(value, hints);
			}
		}
		ExprKind::RecordUpdate { base, fields } => {
			walk_expr(base, hints);
			for (_, value) in fields {
				walk_expr(value, hints);
			}
		}
		ExprKind::If(i) => {
			walk_expr(&i.subject, hints);
			for e in &i.body {
				walk_expr(e, hints);
			}
			if let Some(else_body) = &i.else_body {
				for e in else_body {
					walk_expr(e, hints);
				}
			}
		}
		ExprKind::When(w) => {
			walk_expr(&w.subject, hints);
			for case in &w.cases {
				for e in &case.body {
					walk_expr(e, hints);
				}
			}
		}
		ExprKind::While(w) => {
			walk_expr(&w.subject, hints);
			for e in &w.body {
				walk_expr(e, hints);
			}
		}
		ExprKind::Scope(s) => {
			for e in &s.body {
				walk_expr(e, hints);
			}
		}
		ExprKind::Using { body, .. } => {
			for e in body {
				walk_expr(e, hints);
			}
		}
		ExprKind::Try(t) => {
			if t.binding {
				if let PatternKind::Identifier(id) = &t.pattern.kind {
					push(hints, &id.range, &t.pattern_ty);
				}
			}
			walk_expr(&t.value, hints);
			for e in &t.rest {
				walk_expr(e, hints);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;
	use std::sync::atomic::{AtomicU32, Ordering};

	// Analyze a self-contained snippet (no on-disk imports) and collect the
	// inlay hints. The analyzer needs a real entry path, so we materialize the
	// source to a unique temp file; `use std/...` resolves from the baked-in
	// stdlib, not disk, so single-file fixtures type-check fine.
	fn hints_for(src: &str) -> Vec<InlayHint> {
		static COUNTER: AtomicU32 = AtomicU32::new(0);
		let n = COUNTER.fetch_add(1, Ordering::Relaxed);
		let mut dir: PathBuf = std::env::temp_dir();
		dir.push(format!("pluma-inlay-{}-{}", std::process::id(), n));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("main.pa");
		std::fs::write(&path, src).unwrap();

		let result = crate::analysis::analyze_document(&path, src.as_bytes().to_vec());
		std::fs::remove_dir_all(&dir).ok();
		let module = result.module.expect("analysis produced no module");
		build_hints(&module)
	}

	fn labels(src: &str) -> Vec<String> {
		hints_for(src).into_iter().map(|h| h.label).collect()
	}

	#[test]
	fn infers_let_and_param_types() {
		let src =
			"use std/list\n\ndef main = fun {\n\tlet xs = [1, 2, 3]\n\tlet n = list.length xs\n\tn\n}\n";
		let labels = labels(src);
		assert!(
			labels.contains(&": list int".to_string()),
			"expected `: list int` for xs, got {:?}",
			labels
		);
		assert!(
			labels.contains(&": int".to_string()),
			"expected `: int` for n, got {:?}",
			labels
		);
	}

	#[test]
	fn shows_lambda_param_types() {
		// The lambda's param is pinned to `int` by the `list.map` over `[1,2,3]`.
		let src = "use std/list\n\ndef main = fun {\n\tlist.map [1, 2, 3] (fun x { x })\n}\n";
		let labels = labels(src);
		assert!(
			labels.contains(&": int".to_string()),
			"expected `: int` for the lambda param x, got {:?}",
			labels
		);
	}

	#[test]
	fn suppresses_unresolved_type_variables() {
		// A fully generic identity: the param and the binding are bare type
		// variables, which would render as the useless `: a` — so no hints.
		let src = "def id = fun x {\n\tlet y = x\n\ty\n}\n";
		assert!(
			hints_for(src).is_empty(),
			"generic identity should yield no hints, got {:?}",
			labels(src)
		);
	}

	#[test]
	fn suppresses_annotated_let() {
		// `n` already carries `:: int`; a redundant inlay hint would be noise.
		let src = "use std/list\n\ndef main = fun {\n\tlet n :: int = list.length [1]\n\tn\n}\n";
		assert!(
			!labels(src).iter().any(|l| l == ": int"),
			"annotated let should not get an inlay hint, got {:?}",
			labels(src)
		);
	}

	#[test]
	fn hint_position_is_after_the_binder() {
		let src = "use std/list\n\ndef main = fun {\n\tlet xs = [1, 2, 3]\n\txs\n}\n";
		let hint = hints_for(src)
			.into_iter()
			.find(|h| h.label == ": list int")
			.expect("missing xs hint");
		// `\tlet xs` — line 3 (0-based). The hint is inserted right after `xs`:
		// tab(1) + "let "(4) + "xs"(2) = column 7, the position past the `s`.
		assert_eq!((hint.line, hint.col), (3, 7));
	}
}
