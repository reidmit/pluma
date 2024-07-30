use crate::location::Range;

#[derive(Clone)]
pub struct IdentifierNode {
	pub name: String,
	pub range: Range,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for IdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "ident({:#?}) `{}`", self.range, self.name)
	}
}
