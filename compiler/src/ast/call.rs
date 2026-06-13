use crate::location::Range;
use std::cell::RefCell;
use std::rc::Rc;

use super::*;

#[derive(Clone)]
pub struct CallNode {
	pub range: Range,
	pub callee: Box<ExprNode>,
	pub args: Vec<ExprNode>,
	// Dictionary args to prepend before user args at this call. Populated
	// when the callee is a polymorphic constrained value (e.g. calling
	// `double` whose scheme is `forall a. Numeric a => a -> a`). Each cell
	// is shared with a Class constraint, mutated by the discharge pass.
	pub dict_args: Vec<DispatchCell>,
	// Record-shape monomorphization. When the callee resolves to a generic
	// top-level def, this holds `(qualified def name, the def's generic
	// scheme type)` captured during constrain — the scheme type still carries
	// the def's own quantified vars. After annotate the concrete `callee.ty`
	// is diffed against this scheme to recover the closed type substitution
	// that selects a specialization. `None` for monomorphic / unresolved
	// callees.
	pub mono_callee: Option<(String, crate::types::Type)>,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Resolved {
	// Concrete instance: load the named global slot, which holds the
	// pre-built `Value::MethodDict`.
	Global(String),
	// Polymorphic forwarding: the dict comes from the enclosing function's
	// own dict parameter (`param_idx` is the local slot index).
	Forwarded(u16),
	// Parametric instance applied to inner dispatches.
	// `ctor_slot` is the named global slot holding the instance
	// constructor (a closure that takes N inner dicts and returns a
	// `Value::MethodDict`). `inner` gives one resolution per `where`-clause
	// constraint, evaluated in declaration order.
	InstanceChain {
		ctor_slot: String,
		inner: Vec<Resolved>,
	},
	// The auto-derived `wire` trait. Unlike the other
	// traits, `wire` has no per-type instance dictionaries: its "dictionary"
	// is a *schema descriptor* synthesized from the type's structure. Codegen
	// lowers the shape into a `__prelude__.wire-schema` value (the runtime
	// reification consumed by the `wire-encode`/`wire-decode` builtins). A
	// `Var` leaf in the shape is a type-variable position whose schema is
	// forwarded from a dict parameter (polymorphic `wire a`).
	WireSchema(WireShape),
}

// The compile-time skeleton of a `wire` schema, built from a type's structure
// by `Analyzer::build_wire_shape` and lowered to a runtime `wire-schema` value
// by the backend. Mirrors the `wire-schema` prelude enum.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum WireShape {
	Int,
	Float,
	Bool,
	Str,
	Bytes,
	Duration,
	Nothing,
	List(Box<WireShape>),
	Tuple(Vec<WireShape>),
	// `dict k v` — key schema + value schema. The key is always a primitive
	// (build_wire_shape rejects compound keys), so the codec can rehash on
	// decode without the `hash` instance.
	Dict(Box<WireShape>, Box<WireShape>),
	// Field name + field shape, in a canonical (name-sorted) order shared by
	// both encode and decode.
	Record(Vec<(String, WireShape)>),
	Enum {
		// Fully-qualified enum name, e.g. `__prelude__.option`.
		qualified: String,
		// Variants in declaration order (the index is the wire tag); each
		// carries its name + payload field shapes.
		variants: Vec<(String, Vec<WireShape>)>,
	},
	// A type-variable position: the schema arrives at runtime via a forwarded
	// `wire a` dictionary (itself a `wire-schema` value). Carries the inner
	// dispatch resolution (a `Forwarded`, or a nested resolution).
	Var(Box<Resolved>),
	// A back-reference to a recursive enum (by qualified name) whose inline
	// `Enum` definition is an enclosing ancestor in the shape — cuts the cycle
	// so a recursive type's schema stays finite.
	EnumRef(String),
}

// Typeclass dispatch metadata for an AST site. Shared between the AST
// (which carries the cell) and the corresponding `Class` constraint
// (which holds the same cell). Discharge / generalization writes
// `resolved` through the cell; codegen reads it back via the AST.
pub struct Dispatch {
	pub trait_name: String,
	// `Some(idx)` for a *trait method dispatch* — the site's value is the
	// method at this index in the trait's declaration order. `None` for a
	// *call forwarding dispatch* — the site just needs a dict, not a
	// specific method.
	pub method_idx: Option<usize>,
	// The tyvar that, under the final substitution, gives the dispatch
	// type at this site. Used by the forwarded-dispatch resolver: if the
	// substituted type is a Var matching the enclosing function's bound
	// tyvar, the dispatch is `Forwarded` to that function's dict param.
	// Type::Unknown until set during constrain (when the constraint is
	// emitted).
	pub dispatch_var: crate::types::Type,
	pub resolved: Option<Resolved>,
}

pub type DispatchCell = Rc<RefCell<Dispatch>>;

pub fn new_dispatch(
	trait_name: String,
	method_idx: Option<usize>,
	dispatch_var: crate::types::Type,
) -> DispatchCell {
	Rc::new(RefCell::new(Dispatch {
		trait_name,
		method_idx,
		dispatch_var,
		resolved: None,
	}))
}

// Shared collection of dispatch cells, populated during Gen/Inst
// processing. Used to bridge from a polymorphic value reference (an
// Identifier ExprNode) up to the enclosing CallNode that needs to thread
// dicts as hidden leading args. Conceptually: "the cells this Inst will
// create when matched against its Gen."
pub type DispatchSink = Rc<RefCell<Vec<DispatchCell>>>;

pub fn new_dispatch_sink() -> DispatchSink {
	Rc::new(RefCell::new(Vec::new()))
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for CallNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let mut d = f.debug_struct(&format!("call({:#?})", self.range));
		d.field("callee", &self.callee).field("args", &self.args);
		if !self.dict_args.is_empty() {
			let dicts: Vec<_> = self
				.dict_args
				.iter()
				.map(|c| format!("{:?}", c.borrow().resolved))
				.collect();
			d.field("dict_args", &dicts);
		}
		d.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Dispatch {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"dispatch({}.{:?} -> {:?})",
			self.trait_name, self.method_idx, self.resolved
		)
	}
}
