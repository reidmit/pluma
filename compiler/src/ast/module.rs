use super::*;

pub struct ModuleNode {
	pub span: Span,
	pub body: Vec<DefinitionNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ModuleNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"module({}-{}) {:#?}",
			self.span.0, self.span.1, self.body
		)
	}
}
