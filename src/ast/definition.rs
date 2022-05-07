use super::*;

pub struct DefinitionNode {
	pub pos: Position,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
}

pub enum DefinitionKind {
	Expr(ExprNode),
	// Type(TypeDefNode),
}

impl std::fmt::Debug for DefinitionNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"def:{}-{} ({:?}) ({:#?})",
			self.pos.0, self.pos.1, self.name, self.kind
		)
	}
}

impl std::fmt::Debug for DefinitionKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self {
			DefinitionKind::Expr(expr) => write!(f, "{:#?}", expr),
		}
	}
}
