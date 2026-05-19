use super::*;
use crate::{location::Range, types::*};

pub struct DefinitionNode {
	pub range: Range,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
	pub ty: Type,
	// Number of hidden dictionary parameters codegen prepends to this
	// def's user-facing arity. Equal to the number of class constraints
	// in the def's generalized scheme. Set during the forwarded-dispatch
	// resolution pass.
	pub dict_param_count: u16,
}

pub enum DefinitionKind {
	Expr(ExprNode),
	Alias(TypeExprNode),
	Enum(EnumNode),
	Trait(TraitNode),
	Instance(InstanceNode),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("def({:#?}) :: {}", self.range, self.ty))
			.field("name", &self.name)
			.field("kind", &self.kind)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self {
			DefinitionKind::Expr(expr) => write!(f, "{:#?}", expr),
			DefinitionKind::Alias(ty_expr) => write!(f, "alias {:#?}", ty_expr),
			DefinitionKind::Enum(enum_node) => write!(f, "{:#?}", enum_node),
			DefinitionKind::Trait(trait_node) => write!(f, "{:#?}", trait_node),
			DefinitionKind::Instance(inst_node) => write!(f, "{:#?}", inst_node),
		}
	}
}
