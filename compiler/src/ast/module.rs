use super::*;
use crate::location::Range;

pub struct ModuleNode {
	pub range: Range,
	pub body: Vec<DefinitionNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ModuleNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "module({:#?}) {:#?}", self.range, self.body)
	}
}
