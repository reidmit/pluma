use super::*;

pub struct CallNode {
	pub pos: Position,
	pub callee: Box<ExprNode>,
	pub args: Vec<ExprNode>,
}

impl std::fmt::Debug for CallNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"call:{}:{} {:#?} {:#?}",
			self.pos.0, self.pos.1, self.callee, self.args
		)
	}
}
