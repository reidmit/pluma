use super::*;
use crate::location::Range;

// A typeclass instance declaration: `for TRAIT on TYPE { defs }`. Each
// inner def is a regular `def NAME fun ARGS { BODY }` providing one of the
// trait's methods. Currently the head is a plain type name; generic
// application heads (`option a`) and `where` clauses are a future
// generalization.
pub struct InstanceNode {
	pub range: Range,
	pub trait_name: IdentifierNode,
	// The type the instance is declared on. Currently this is a simple
	// type name (TypeExprNode lets a future generalization reuse the slot for
	// generic application heads without reshaping the AST).
	pub head: TypeExprNode,
	// `where (constraint, ...)` clause for parametric instances. Each
	// constraint is `TRAIT_NAME TYPE_PARAM` — the type param must be one
	// of the head's free type variables.
	pub where_clause: Vec<InstanceConstraintNode>,
	pub methods: Vec<DefinitionNode>,
	// Set by the analyzer once the instance is registered: the global
	// slot name (concrete instances) or the constructor function's slot
	// name (parametric instances). Used by codegen to wire up
	// `Resolved::Global` references.
	pub instance_slot_name: String,
	// The method names in the trait's declaration order — instances may
	// declare methods in any order, but codegen builds the `Value::MethodDict`
	// in this canonical order.
	pub canonical_method_order: Vec<String>,
}

// A single constraint inside an instance's `where` clause:
// `where (showable a)` parses to `InstanceConstraintNode { trait_name:
// "showable", param: "a" }`.
pub struct InstanceConstraintNode {
	pub range: Range,
	pub trait_name: IdentifierNode,
	pub param: IdentifierNode,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for InstanceConstraintNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{} {}", self.trait_name.name, self.param.name)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for InstanceNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("instance({:#?})", self.range))
			.field("trait", &self.trait_name)
			.field("head", &self.head)
			.field("methods", &self.methods)
			.finish()
	}
}
