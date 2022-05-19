use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ModuleNode {
	pub loc: Location,
	pub body: Vec<DefinitionNode>,
}
