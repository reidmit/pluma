use compiler::ast::*;
use compiler::types::Type;
use compiler::{Module, Range};

// A precomputed lookup entry: "if the cursor lands inside this range,
// show this type". Built eagerly at analysis time so the lookup itself
// is just a linear scan over Send-only data. `doc` carries the doc-comment
// block for a top-level def name (rendered below the type on hover).
#[derive(Clone)]
pub struct HoverHit {
	pub range: Range,
	pub ty: Type,
	pub doc: Option<String>,
}

pub fn build_index(module: &Module) -> Vec<HoverHit> {
	let mut hits = Vec::new();
	if let Some(ast) = module.ast.as_ref() {
		for def in &ast.body {
			let doc = doc_comment_for(module, def.range.start.line);
			walk_def(def, &mut hits, doc);
		}
	}
	hits
}

// The doc comment for a top-level def: the contiguous run of full-line
// comments directly above it. We bound the search at the previous
// top-level item's end line so a trailing comment on the line above
// (e.g. `def prev = 1 # note`) is never mistaken for this def's doc.
fn doc_comment_for(module: &Module, def_start_line: usize) -> Option<String> {
	let ast = module.ast.as_ref()?;

	let mut floor: isize = -1;
	for u in &ast.uses {
		if u.range.end.line < def_start_line {
			floor = floor.max(u.range.end.line as isize);
		}
	}
	for d in &ast.body {
		if d.range.end.line < def_start_line {
			floor = floor.max(d.range.end.line as isize);
		}
	}

	// Walk upward from the line just above the def, collecting comments
	// until a non-comment line (or the previous item) stops the run.
	let mut lines: Vec<String> = Vec::new();
	let mut line = def_start_line as isize - 1;
	while line > floor {
		let Some(text) = module.comments.get(&(line as usize)) else {
			break;
		};
		// Comment text is everything after `#`; drop the conventional
		// single leading space so `# foo` renders as `foo`.
		lines.push(
			text
				.strip_prefix(' ')
				.unwrap_or(text)
				.trim_end()
				.to_string(),
		);
		line -= 1;
	}

	if lines.is_empty() {
		return None;
	}
	lines.reverse();
	Some(lines.join("\n"))
}

pub fn lookup(hits: &[HoverHit], line: u32, character: u32) -> Option<&HoverHit> {
	let l = line as usize;
	let c = character as usize;
	hits
		.iter()
		.filter(|h| contains(&h.range, l, c))
		.min_by_key(|h| range_size(&h.range))
}

// The doc comment to show on hover at a position. Docs live on the def
// name's hit, so hovering the def shows it directly. At a *usage*, we
// resolve the identifier to its definition (via goto's scope-aware
// resolution) and borrow that def's doc — so a usage of `helper` shows
// `helper`'s doc, while a local that shadows a top-level def correctly
// shows nothing.
pub fn doc_for_hover(
	hits: &[HoverHit],
	source: &[u8],
	line: u32,
	character: u32,
) -> Option<String> {
	if let Some(hit) = lookup(hits, line, character) {
		if hit.doc.is_some() {
			return hit.doc.clone();
		}
	}
	let target = crate::goto::goto_definition(source, line, character)?;
	lookup(hits, target.start.line as u32, target.start.col as u32)?
		.doc
		.clone()
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
	hits.push(HoverHit {
		range,
		ty,
		doc: None,
	});
}

// Record a def name's hit, carrying its doc comment. Unlike `record`, this
// keeps the hit even when the type is unknown (e.g. analysis failed
// upstream) so the doc still shows.
fn record_name(hits: &mut Vec<HoverHit>, range: Range, ty: Type, doc: Option<String>) {
	if doc.is_none() && matches!(ty, Type::Unknown) {
		return;
	}
	hits.push(HoverHit { range, ty, doc });
}

fn walk_def(def: &DefinitionNode, hits: &mut Vec<HoverHit>, doc: Option<String>) {
	// Def name itself: show its type, plus its doc comment. For value defs
	// the body expr carries the real inferred type — `def.ty` is left an
	// unconstrained var by the analyzer and resolves to `nothing`, so use
	// the body's type instead (e.g. `int -> int`, not `nothing`).
	let name_ty = match &def.kind {
		DefinitionKind::Expr(expr) => expr.ty.clone(),
		_ => def.ty.clone(),
	};
	record_name(hits, def.name.range, name_ty, doc);

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
				walk_def(method, hits, None);
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

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	// Build the hover index from parser output alone (no analyzer; types
	// come back Unknown, which is fine — these tests only check docs).
	fn doc_at(src: &str, line: u32, character: u32) -> Option<String> {
		let mut module = Module::new("<test>".to_string(), PathBuf::new());
		let mut diags: Vec<compiler::Diagnostic> = Vec::new();
		module.parse_from_bytes(src.as_bytes().to_vec(), &mut diags);
		let hits = build_index(&module);
		lookup(&hits, line, character).and_then(|h| h.doc.clone())
	}

	// The doc shown on hover, resolving usages through to their definition.
	fn hover_doc_at(src: &str, line: u32, character: u32) -> Option<String> {
		let mut module = Module::new("<test>".to_string(), PathBuf::new());
		let mut diags: Vec<compiler::Diagnostic> = Vec::new();
		module.parse_from_bytes(src.as_bytes().to_vec(), &mut diags);
		let hits = build_index(&module);
		doc_for_hover(&hits, src.as_bytes(), line, character)
	}

	#[test]
	fn contiguous_block_above_def() {
		let src = "# greet someone\n# politely\ndef greet = fun name { name }\n";
		// Hover the def name `greet` (line 2, col 4).
		assert_eq!(
			doc_at(src, 2, 4),
			Some("greet someone\npolitely".to_string())
		);
	}

	#[test]
	fn single_leading_space_stripped() {
		let src = "#no space\ndef x = 1\n";
		assert_eq!(doc_at(src, 1, 4), Some("no space".to_string()));
	}

	#[test]
	fn blank_line_breaks_adjacency() {
		// A blank line between the comment and the def means it's not a doc.
		let src = "# not a doc\n\ndef x = 1\n";
		assert_eq!(doc_at(src, 2, 4), None);
	}

	#[test]
	fn trailing_comment_on_prev_def_not_captured() {
		// The comment is a trailing comment on `a`'s line, not a doc for `b`.
		let src = "def a = 1 # trailing\ndef b = 2\n";
		assert_eq!(doc_at(src, 1, 4), None);
	}

	#[test]
	fn no_comment_means_no_doc() {
		let src = "def x = 1\n";
		assert_eq!(doc_at(src, 0, 4), None);
	}

	#[test]
	fn doc_shows_at_usage() {
		let src = "# greet someone\ndef greet = fun { 1 }\ndef main = fun {\n\tgreet ()\n}\n";
		// Hovering the `greet` call on line 3 (col 1, after the tab) surfaces
		// greet's doc, resolved through the usage.
		assert_eq!(hover_doc_at(src, 3, 1), Some("greet someone".to_string()));
	}

	#[test]
	fn shadowing_local_shows_no_doc() {
		// A param `x` shadows top-level `def x` (which has a doc). Hovering the
		// param usage must not borrow the top-level def's doc.
		let src = "# the global x\ndef x = 1\ndef f = fun x {\n\tx\n}\n";
		assert_eq!(hover_doc_at(src, 3, 1), None);
		// But hovering the top-level def name still shows its doc.
		assert_eq!(hover_doc_at(src, 1, 4), Some("the global x".to_string()));
	}
}
