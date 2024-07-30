use super::*;
use crate::{location::Range, types::*};

pub struct DefinitionNode {
	pub range: Range,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
	pub ty: Type,
}

pub enum DefinitionKind {
	Expr(ExprNode),
	Alias(TypeExprNode),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("def({:#?}) :: {}", self.range, self.ty))
			.field("name", &self.name)
			.field("kind", &self.kind)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self {
			DefinitionKind::Expr(expr) => write!(f, "{:#?}", expr),
			DefinitionKind::Alias(ty_expr) => write!(f, "alias {:#?}", ty_expr),
		}
	}
}
