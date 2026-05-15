use super::*;
use crate::location::Range;

pub struct EnumNode {
	pub range: Range,
	pub variants: Vec<EnumVariantNode>,
}

pub struct EnumVariantNode {
	pub range: Range,
	pub name: IdentifierNode,
	pub params: Option<Vec<TypeExprNode>>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for EnumNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "enum({:#?}) {:#?}", self.range, self.variants)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for EnumVariantNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("variant({:#?})", self.range))
			.field("name", &self.name)
			.field("params", &self.params)
			.finish()
	}
}
