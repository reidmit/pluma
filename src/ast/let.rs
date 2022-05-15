use super::*;

pub struct LetNode {
	pub pos: Position,
	pub name: IdentifierNode,
	pub value: Box<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LetNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"let:{}-{} ({:?}) ({:#?})",
			self.pos.0, self.pos.1, self.name, self.value
		)
	}
}
