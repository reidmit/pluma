use super::*;
use crate::expr_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DefinitionNode {
	pub span: Span,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
	pub inferred_type: ExprType,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum DefinitionKind {
	Expr(ExprNode),
	Alias(TypeExprNode),
}
