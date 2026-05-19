use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct WhileNode {
	pub range: Range,
	pub subject: Box<ExprNode>,
	pub pattern: PatternNode,
	pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for WhileNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"while({:#?}) {:#?} is {:#?} {:#?}",
			self.range, self.subject, self.pattern, self.body
		)
	}
}
