use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct IfNode {
	pub range: Range,
	pub subject: Box<ExprNode>,
	pub pattern: PatternNode,
	pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for IfNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"if({:#?}) {:#?} {:#?} {:#?}",
			self.range, self.subject, self.pattern, self.body
		)
	}
}
