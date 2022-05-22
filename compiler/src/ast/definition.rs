use super::*;
use crate::typing::*;

pub struct DefinitionNode {
	pub span: Span,
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
		f.debug_struct(&format!(
			"def({}-{}) :: {}",
			self.span.0, self.span.1, self.ty
		))
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
			DefinitionKind::Alias(ty_expr) => write!(f, "{:#?}", ty_expr),
		}
	}
}
