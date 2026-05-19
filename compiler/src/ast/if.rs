use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct IfNode {
	pub range: Range,
	pub subject: Box<ExprNode>,
	pub pattern: PatternNode,
	pub body: Vec<ExprNode>,
	pub else_body: Option<Vec<ExprNode>>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for IfNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.else_body {
			Some(else_body) => write!(
				f,
				"if({:#?}) {:#?} {:#?} {:#?} else {:#?}",
				self.range, self.subject, self.pattern, self.body, else_body
			),
			None => write!(
				f,
				"if({:#?}) {:#?} {:#?} {:#?}",
				self.range, self.subject, self.pattern, self.body
			),
		}
	}
}
