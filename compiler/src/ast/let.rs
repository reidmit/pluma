use super::*;

pub struct LetNode {
	pub span: Span,
	pub name: IdentifierNode,
	pub value: Box<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LetNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("let({}-{})", self.span.0, self.span.1))
			.field("name", &self.name)
			.field("value", &self.value)
			.finish()
	}
}
