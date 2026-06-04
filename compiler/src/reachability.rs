// FULLSTACK Layer 2 — def-level reachability for target gating.
//
// Coarse module-import gating (`platform::gate` at the `use` site) is too
// blunt for fullstack: a shared module's `remote def` body may legitimately
// call server-only modules (`std.sys.*`) even when the module is reachable
// from a web client — the body is a *server island*, compiled only for the
// server. The client closure must stop at `remote def` bodies.
//
// So gating walks reachability at def granularity. A module's imports are
// followed only when referenced by its **non-remote** code: an import used
// *only* inside `remote def` bodies is an island dependency and is not pulled
// into the closure. The island rule isn't special-cased — it falls out of
// skipping `remote def` bodies when collecting a module's live references.

use crate::ast::{DefinitionKind, ExprKind, ExprNode, ListItem, ModuleNode};
use std::collections::HashSet;

// The set of imported-namespace local names a module's **non-remote** code
// references (`io` for `std.sys.io`, etc.). Imports outside this set are only
// used in `remote def` bodies (or unused), so they don't enter the closure.
pub fn live_prefixes(ast: &ModuleNode) -> HashSet<String> {
	let mut out = HashSet::new();
	for def in &ast.body {
		if def.is_remote {
			continue;
		}
		if let DefinitionKind::Expr(body) = &def.kind {
			collect_namespace_prefixes(body, &mut out);
		}
	}
	out
}

// Collect the leading segment of every `NamespaceAccess` in `expr` — the local
// namespace name (`list`, `io`, …) that a `use` binds. Walks the same child
// shape the lowerer does, so no referenced namespace is missed.
fn collect_namespace_prefixes(expr: &ExprNode, out: &mut HashSet<String>) {
	use ExprKind::*;
	match &expr.kind {
		NamespaceAccess(path) => {
			if let Some(first) = path.first() {
				out.insert(first.name.clone());
			}
		}
		BinaryOperation { left, right, .. } => {
			collect_namespace_prefixes(left, out);
			collect_namespace_prefixes(right, out);
		}
		UnaryOperation { right, .. } => collect_namespace_prefixes(right, out),
		ElementAccess { receiver, .. } => collect_namespace_prefixes(receiver, out),
		FieldAccess { receiver, .. } => collect_namespace_prefixes(receiver, out),
		Fun(f) => collect_block(&f.body, out),
		Call(c) => {
			collect_namespace_prefixes(&c.callee, out);
			for a in &c.args {
				collect_namespace_prefixes(a, out);
			}
		}
		Grouping(inner) | Defer(inner) => collect_namespace_prefixes(inner, out),
		Interpolation(parts) => collect_block(parts, out),
		Let(l) => collect_namespace_prefixes(&l.value, out),
		Record(fields) => {
			for (_, e) in fields {
				collect_namespace_prefixes(e, out);
			}
		}
		RecordUpdate { base, fields } => {
			collect_namespace_prefixes(base, out);
			for (_, e) in fields {
				collect_namespace_prefixes(e, out);
			}
		}
		Tuple(items) => collect_block(items, out),
		Try(t) => {
			collect_namespace_prefixes(&t.value, out);
			collect_block(&t.rest, out);
		}
		List(items) => {
			for it in items {
				let (ListItem::Item(e) | ListItem::Spread(e)) = it;
				collect_namespace_prefixes(e, out);
			}
		}
		If(n) => {
			collect_namespace_prefixes(&n.subject, out);
			collect_block(&n.body, out);
			if let Some(eb) = &n.else_body {
				collect_block(eb, out);
			}
		}
		When(n) => {
			collect_namespace_prefixes(&n.subject, out);
			for c in &n.cases {
				collect_block(&c.body, out);
			}
		}
		While(n) => {
			collect_namespace_prefixes(&n.subject, out);
			collect_block(&n.body, out);
		}
		Scope(n) => collect_block(&n.body, out),
		Identifier(_) | Literal(_) | EmptyTuple | Regex(_) | Builtin(_) => {}
	}
}

fn collect_block(block: &[ExprNode], out: &mut HashSet<String>) {
	for e in block {
		collect_namespace_prefixes(e, out);
	}
}
