use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct LetNode {
	pub range: Range,
	pub pattern: PatternNode,
	pub value: Box<ExprNode>,
	// Optional binding type annotation: `let name :: TYPE = expr`.
	// When present, the analyzer unifies the value's inferred type
	// with this annotation — mirrors the top-level `def` form.
	pub type_annotation: Option<TypeExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LetNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// `type_annotation` is intentionally omitted — the value's
		// inferred type on `value` already reflects the annotation
		// (after unification) and the syntactic node would just clutter
		// snapshots. Mirrors `DefinitionNode`'s Debug impl.
		f.debug_struct(&format!("let({:#?})", self.range))
			.field("pattern", &self.pattern)
			.field("value", &self.value)
			.finish()
	}
}
