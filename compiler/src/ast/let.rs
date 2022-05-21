use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct LetNode {
	pub span: Span,
	pub name: IdentifierNode,
	pub value: Box<ExprNode>,
}
