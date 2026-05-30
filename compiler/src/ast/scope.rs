use super::*;
use crate::location::Range;

/// `scope (as IDENT)? { body }` and `manual scope as IDENT { body }` — the
/// structured-concurrency block. The body is a block of
/// statements (like a function body, so `try`/`let`/`defer` all work) that
/// must produce a `task a`; the whole `scope` expression has that type.
///
/// `handle` is the scope-handle name bound by `as` (the `s` in `scope as s`);
/// inside the body, `s.spawn` / `s.cancel ()` / `s.next ()` / `s.cancel-after`
/// are the handle methods. A bare `scope { ... }` has no handle (you can still
/// `try` inside, just not spawn). `manual` selects the non-fail-fast form.
///
/// The analyzer rewrites a `Scope` into a call to the hidden `task.scope-new`
/// kernel builtin wrapping the body in a `fun handle { body }` closure, so
/// codegen sees an ordinary call (see `analyzer::constrain_expr`).
#[derive(Clone)]
pub struct ScopeNode {
	pub range: Range,
	pub manual: bool,
	pub handle: Option<IdentifierNode>,
	pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ScopeNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("scope({:#?})", self.range))
			.field("manual", &self.manual)
			.field("handle", &self.handle)
			.field("body", &self.body)
			.finish()
	}
}
