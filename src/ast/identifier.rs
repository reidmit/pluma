use super::*;

#[derive(Clone)]
pub struct IdentifierNode {
	pub pos: Position,
	pub name: String,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for IdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "ident:{}-{} `{}`", self.pos.0, self.pos.1, self.name)
	}
}
