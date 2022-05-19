use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeIdentifierNode {
	pub span: Span,
	pub name: String,
	pub generics: Vec<TypeExprNode>,
}
