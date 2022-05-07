use super::*;

pub struct LambdaNode {
	pub pos: Position,
	pub params: Vec<IdentifierNode>,
	pub body: Vec<ExprNode>,
}

impl std::fmt::Debug for LambdaNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"lambda:{}:{} {:#?} {:#?}",
			self.pos.0, self.pos.1, self.params, self.body
		)
	}
}
