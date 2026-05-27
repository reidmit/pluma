use super::*;
use crate::location::Range;
use crate::types::Type;

// `try Pattern = Expr` followed by the rest of the enclosing block.
//
// At parse time `rest` carries everything after the `=` line through to
// the end of the surrounding body (`parse_body_expressions` collects it
// when it sees the `try` keyword). At analyze time the analyzer peeks
// the RHS's inferred head constructor and rewrites this node into a
// `<carrier>.then value fun pattern { rest }` call.
//
// `pattern_ty` is the fresh tyvar the analyzer binds `pattern` to during
// the first constraint-gen pass. Stored on the node so the post-unify
// dispatch pass can link it to the resolved carrier's payload type
// (e.g. `α := int` when the RHS is `option int`).
//
// `task_carrier`: the `option`/`result` carriers are rewritten away by the
// analyzer into `<carrier>.then` calls, so a `Try` node normally never
// survives to codegen. The `task` carrier is the exception — it is left
// intact (with `task_carrier = true`) so codegen can lower the whole
// `try`-chain into a resumable state machine (the CPS transform), rather
// than into a tree of separately-allocated continuation closures. See
// `do_try_dispatch` in the analyzer and `emit_async_*` in codegen.
#[derive(Clone)]
pub struct TryNode {
	pub range: Range,
	pub pattern: PatternNode,
	pub value: Box<ExprNode>,
	pub rest: Vec<ExprNode>,
	pub pattern_ty: Type,
	pub task_carrier: bool,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TryNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("try({:#?})", self.range))
			.field("pattern", &self.pattern)
			.field("value", &self.value)
			.field("rest", &self.rest)
			.finish()
	}
}
