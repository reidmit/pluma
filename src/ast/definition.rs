use super::*;
use crate::expr_type::*;

pub struct DefinitionNode {
	pub pos: Position,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
	pub inferred_type: ExprType,
}

pub enum DefinitionKind {
	Expr(ExprNode),
	Alias(TypeExprNode),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"def:{}-{}::{} ({:?}) ({:#?})",
			self.pos.0, self.pos.1, self.inferred_type, self.name, self.kind
		)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self {
			DefinitionKind::Expr(expr) => write!(f, "{:#?}", expr),
			DefinitionKind::Alias(type_expr) => write!(f, "alias {:#?}", type_expr),
		}
	}
}
