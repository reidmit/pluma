use super::*;
use crate::location::Range;

pub struct TypeIdentifierNode {
	pub range: Range,
	pub name: String,
	pub generics: Vec<TypeExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TypeIdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "type-ident({:#?}) `{}`", self.range, self.name)
	}
}
