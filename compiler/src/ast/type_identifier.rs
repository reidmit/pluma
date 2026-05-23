use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct TypeIdentifierNode {
	pub range: Range,
	// Optional module namespace prefix: `Some(ident)` for `module.TypeName`,
	// `None` for a bare `TypeName`.
	pub module: Option<IdentifierNode>,
	pub name: String,
	pub generics: Vec<TypeExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TypeIdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.module {
			Some(module) => write!(
				f,
				"type-ident({:#?}) `{}.{}`",
				self.range, module.name, self.name
			),
			None => write!(f, "type-ident({:#?}) `{}`", self.range, self.name),
		}
	}
}
