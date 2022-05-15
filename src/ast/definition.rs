use super::*;

pub struct DefinitionNode {
	pub pos: Position,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
	pub doc_comment_lines: std::ops::Range<usize>,
}

pub enum DefinitionKind {
	Expr(ExprNode),
	// Type(TypeDefNode),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"def:{}-{} ({:?}) ({:#?})",
			self.pos.0, self.pos.1, self.name, self.kind
		)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self {
			DefinitionKind::Expr(expr) => write!(f, "{:#?}", expr),
		}
	}
}
