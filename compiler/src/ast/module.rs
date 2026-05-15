use super::*;
use crate::location::Range;

pub struct ModuleNode {
	pub range: Range,
	pub uses: Vec<UseNode>,
	pub body: Vec<DefinitionNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ModuleNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if self.uses.is_empty() {
			write!(f, "module({:#?}) {:#?}", self.range, self.body)
		} else {
			write!(
				f,
				"module({:#?}) {:#?} {:#?}",
				self.range, self.uses, self.body
			)
		}
	}
}
