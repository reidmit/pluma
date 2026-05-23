use compiler::ast::*;
use compiler::types::Type;
use compiler::{Module, Range};

// A precomputed lookup entry: "if the cursor lands inside this range,
// show this type". Built eagerly at analysis time so the lookup itself
// is just a linear scan over Send-only data.
#[derive(Clone)]
pub struct HoverHit {
	pub range: Range,
	pub ty: Type,
}

pub fn build_index(module: &Module) -> Vec<HoverHit> {
	let mut hits = Vec::new();
	if let Some(ast) = module.ast.as_ref() {
		for def in &ast.body {
			walk_def(def, &mut hits);
		}
	}
	hits
}

pub fn lookup(hits: &[HoverHit], line: u32, character: u32) -> Option<&HoverHit> {
	let l = line as usize;
	let c = character as usize;
	hits
		.iter()
		.filter(|h| contains(&h.range, l, c))
		.min_by_key(|h| range_size(&h.range))
}

fn range_size(r: &Range) -> usize {
	// Lines weighted heavily so a multi-line range never beats a
	// single-line one that contains the same point.
	let lines = r.end.line.saturating_sub(r.start.line);
	let cols = if r.start.line == r.end.line {
		r.end.col.saturating_sub(r.start.col)
	} else {
		r.end.col
	};
	lines * 10_000 + cols
}

fn contains(r: &Range, line: usize, character: usize) -> bool {
	if line < r.start.line || line > r.end.line {
		return false;
	}
	if line == r.start.line && character < r.start.col {
		return false;
	}
	if line == r.end.line && character > r.end.col {
		return false;
	}
	true
}

fn record(hits: &mut Vec<HoverHit>, range: Range, ty: Type) {
	if matches!(ty, Type::Unknown) {
		return;
	}
	hits.push(HoverHit { range, ty });
}

fn walk_def(def: &DefinitionNode, hits: &mut Vec<HoverHit>) {
	// Def name itself: show the def's full type.
	record(hits, def.name.range, def.ty.clone());

	match &def.kind {
		DefinitionKind::Expr(expr) => walk_expr(expr, hits),
		DefinitionKind::Alias(_) => {
			// Alias bodies are type expressions — no per-node inferred
			// types to surface yet.
		}
		DefinitionKind::Enum(en) => {
			for variant in &en.variants {
				record(hits, variant.name.range, def.ty.clone());
			}
		}
		DefinitionKind::Trait(t) => {
			for m in &t.methods {
				record(hits, m.name.range, def.ty.clone());
				if let Some(default) = &m.default {
					walk_expr(default, hits);
				}
			}
		}
		DefinitionKind::Instance(inst) => {
			for method in &inst.methods {
				walk_def(method, hits);
			}
		}
		DefinitionKind::Test { body, .. } => {
			for stmt in body {
				walk_expr(stmt, hits);
			}
		}
	}
}

fn walk_expr(expr: &ExprNode, hits: &mut Vec<HoverHit>) {
	record(hits, expr.range, expr.ty.clone());

	match &expr.kind {
		ExprKind::Identifier(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::EmptyTuple
		| ExprKind::Builtin(_) => {}
		ExprKind::BinaryOperation { left, right, .. } => {
			walk_expr(left, hits);
			walk_expr(right, hits);
		}
		ExprKind::UnaryOperation { right, .. } => walk_expr(right, hits),
		ExprKind::ElementAccess { receiver, .. } => walk_expr(receiver, hits),
		ExprKind::FieldAccess { receiver, .. } => walk_expr(receiver, hits),
		ExprKind::Fun(f) => walk_fun(f, hits),
		ExprKind::Call(c) => {
			walk_expr(&c.callee, hits);
			for arg in &c.args {
				walk_expr(arg, hits);
			}
		}
		ExprKind::Grouping(inner) => walk_expr(inner, hits),
		ExprKind::Interpolation(parts) | ExprKind::Tuple(parts) => {
			for p in parts {
				walk_expr(p, hits);
			}
		}
		ExprKind::List(items) => {
			for item in items {
				walk_expr(item.expr(), hits);
			}
		}
		ExprKind::Let(l) => {
			// Only top-level identifier patterns get a direct hover hit (the
			// whole pattern's type is the value's type). For destructured
			// patterns, hover info for the inner bindings shows up at use
			// sites instead.
			if let PatternKind::Identifier(id) = &l.pattern.kind {
				record(hits, id.range, l.value.ty.clone());
			}
			walk_expr(&l.value, hits);
		}
		ExprKind::Record(fields) => {
			for (_, value) in fields {
				walk_expr(value, hits);
			}
		}
		ExprKind::If(i) => {
			walk_expr(&i.subject, hits);
			for e in &i.body {
				walk_expr(e, hits);
			}
			if let Some(else_body) = &i.else_body {
				for e in else_body {
					walk_expr(e, hits);
				}
			}
		}
		ExprKind::When(w) => {
			walk_expr(&w.subject, hits);
			for case in &w.cases {
				for e in &case.body {
					walk_expr(e, hits);
				}
			}
		}
		ExprKind::While(w) => {
			walk_expr(&w.subject, hits);
			for e in &w.body {
				walk_expr(e, hits);
			}
		}
		ExprKind::NamespaceAccess(_) => {
			// The whole-expr hover hit (recorded above) carries the resolved
			// type. The path segments aren't values, so nothing else to walk.
		}
		ExprKind::Try(t) => {
			// Mirror let's pattern handling: record a hit on the pattern
			// identifier (typed as the carrier's payload), then walk the
			// RHS and the rest of the body.
			if let PatternKind::Identifier(id) = &t.pattern.kind {
				record(hits, id.range, t.pattern_ty.clone());
			}
			walk_expr(&t.value, hits);
			for e in &t.rest {
				walk_expr(e, hits);
			}
		}
	}
}

fn walk_fun(f: &FunNode, hits: &mut Vec<HoverHit>) {
	for p in &f.params {
		record(hits, p.ident.range, p.ty.clone());
	}
	for e in &f.body {
		walk_expr(e, hits);
	}
}
