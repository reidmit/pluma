use super::*;
use crate::location::Range;

pub struct WhenNode {
	pub range: Range,
	pub subject: Box<ExprNode>,
	pub cases: Vec<CaseNode>,
}

pub struct CaseNode {
	pub range: Range,
	pub pattern: PatternNode,
	pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for WhenNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "when({:#?}) {:#?}", self.range, self.cases)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for CaseNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"case({:#?}) is {:#?} {:#?}",
			self.range, self.pattern, self.body
		)
	}
}
