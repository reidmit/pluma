use super::*;
use crate::location::Range;

// A typeclass declaration. The body is a list of method signatures
// (`add fun (a, a) -> a`) plus optional `default` bodies that fall back to
// other methods. The trait's type parameter (`a`) is bound in the body.
pub struct TraitNode {
	pub range: Range,
	pub param: IdentifierNode,
	pub methods: Vec<TraitMethodNode>,
}

pub struct TraitMethodNode {
	pub range: Range,
	pub name: IdentifierNode,
	pub signature: TypeExprNode,
	// If present, the default body to use when an instance omits this method.
	// Body is a `fun ARGS { BODY }` expression.
	pub default: Option<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TraitNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("trait({:#?})", self.range))
			.field("param", &self.param)
			.field("methods", &self.methods)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TraitMethodNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("method({:#?})", self.range))
			.field("name", &self.name)
			.field("signature", &self.signature)
			.field("default", &self.default)
			.finish()
	}
}
