use super::*;
use crate::{location::Range, types::*};

// Whether (and how) a top-level definition is visible to modules that
// `use` this one. Definitions are private by default; the `public` and
// `opaque` keywords widen that. `Opaque` is only valid on enums — it
// exports the type name while withholding its constructors, so importers
// can name the type but can't construct or pattern-match its values.
#[derive(Copy, Clone, PartialEq, Eq, Default)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Visibility {
	#[default]
	Private,
	Opaque,
	Public,
}

pub struct DefinitionNode {
	pub range: Range,
	pub name: IdentifierNode,
	pub kind: DefinitionKind,
	pub visibility: Visibility,
	// Whether this def is an RPC endpoint, marked with the `remote` modifier
	// (`public remote def`). A `remote def` is a server-target island: its
	// signature is the client/server contract, its body is compiled only for
	// the server, and the client closure stops at it (FULLSTACK.md Layer 2).
	// Only valid on value defs; the parser rejects it elsewhere.
	pub is_remote: bool,
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
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for DefinitionNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// Render the visibility keyword only when non-default, so private
		// defs (the common case) keep their existing snapshot shape.
		let vis = match self.visibility {
			Visibility::Private => "",
			Visibility::Opaque => "opaque ",
			Visibility::Public => "public ",
		};
		let remote = if self.is_remote { "remote " } else { "" };
		f.debug_struct(&format!(
			"{}{}def({:#?}) :: {}",
			vis, remote, self.range, self.ty
		))
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
