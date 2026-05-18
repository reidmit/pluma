use super::*;
use crate::location::Range;

pub struct EnumNode {
	pub range: Range,
	// Declared type parameters, e.g. `a` and `b` in `def opt enum a b { ... }`.
	// Variant params can reference these names; the analyzer resolves them to
	// fresh type vars during the def's first pass.
	pub params: Vec<IdentifierNode>,
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
		if self.params.is_empty() {
			write!(f, "enum({:#?}) {:#?}", self.range, self.variants)
		} else {
			write!(
				f,
				"enum({:#?}) <{}> {:#?}",
				self.range,
				self
					.params
					.iter()
					.map(|p| p.name.clone())
					.collect::<Vec<_>>()
					.join(", "),
				self.variants
			)
		}
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
