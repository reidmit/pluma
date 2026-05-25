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
	// Top-level type annotation: `def name :: TYPE = expr`. Only
	// populated on value defs (`DefinitionKind::Expr`). The annotation
	// is the contract — analysis emits a constraint unifying the body's
	// inferred type with the annotated type.
	pub type_annotation: Option<TypeExprNode>,
	// Class constraints declared with a `where (trait param, ...)` clause
	// on the def's signature: `def name :: TYPE where (hash k) = expr`.
	// Each `param` must be a free type variable of the annotation. The
	// analyzer turns these into exported `value_constraints` so call sites
	// thread a dictionary, exactly like the auto-discovered forwarded
	// dispatches. Empty for the common unconstrained case.
	pub where_clause: Vec<InstanceConstraintNode>,
}

pub enum DefinitionKind {
	Expr(ExprNode),
	Alias(TypeExprNode),
	Enum(EnumNode),
	Trait(TraitNode),
	Instance(InstanceNode),
	// `test "description" { body }` — a top-level test block. The
	// description is the human-readable name shown by `pluma test`.
	// The body is a statement list (like a fun body); its final
	// expression's type must be `nothing`. Tests do not register a
	// value binding in the module's scope.
	Test {
		description: String,
		body: Vec<ExprNode>,
	},
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
			DefinitionKind::Test { description, body } => {
				write!(f, "test {:?} {:#?}", description, body)
			}
		}
	}
}
