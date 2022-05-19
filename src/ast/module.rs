use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ModuleNode {
	pub span: Span,
	pub body: Vec<DefinitionNode>,
}
