use super::*;
use crate::location::Range;

pub struct UseNode {
	pub range: Range,
	// dotted module path; e.g. `use sub.utils` produces [sub, utils]
	pub path: Vec<IdentifierNode>,
}

impl UseNode {
	pub fn module_name(&self) -> String {
		self.path.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(".")
	}

	pub fn last_segment(&self) -> &IdentifierNode {
		self.path.last().expect("use path must have at least one segment")
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for UseNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "use({:#?}) `{}`", self.range, self.module_name())
	}
}
