use super::*;

#[derive(Clone)]
pub struct IdentifierNode {
	pub name: String,
	pub span: Span,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for IdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "ident({}-{}) `{}`", self.span.0, self.span.1, self.name)
	}
}
