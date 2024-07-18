use super::*;

pub struct TypeIdentifierNode {
	pub span: Span,
	pub name: String,
	pub generics: Vec<TypeExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TypeIdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"type-ident({}-{}) `{}`",
			self.span.0, self.span.1, self.name
		)
	}
}
