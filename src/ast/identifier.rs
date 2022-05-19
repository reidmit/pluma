use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct IdentifierNode {
	pub span: Span,
	pub name: String,
}
