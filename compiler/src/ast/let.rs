use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct LetNode {
	pub range: Range,
	pub pattern: PatternNode,
	pub value: Box<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LetNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("let({:#?})", self.range))
			.field("pattern", &self.pattern)
			.field("value", &self.value)
			.finish()
	}
}
