use super::*;

pub struct ModuleNode {
	pub pos: Position,
	pub body: Vec<DefinitionNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ModuleNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "module:{}-{} {:#?}", self.pos.0, self.pos.1, self.body)
	}
}
