use crate::ast::{DispatchCell, DispatchSink};
use crate::{location::Range, types::*};

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Constraint {
	Eq(Type, Type, ConstraintReason),
	Gen(Scheme, Type),
	// `Inst(scheme_var_id, target_ty, dispatch_sink)`. When this is
	// resolved against a matching `Gen`, fresh tyvars + class constraints
	// are minted; the cells of the fresh class constraints are pushed
	// into `dispatch_sink` so the surrounding Call can read them as its
	// `dict_args`.
	Inst(usize, Type, DispatchSink, Range),
	// `Class { name, ty, reason }` asserts that `ty` is an instance of the
	// typeclass named `name`. Emitted by `constrain` when resolving trait
	// methods; processed by `discharge` after `unify`.
	Class(ClassConstraint),
}

#[derive(Clone)]
pub struct ClassConstraint {
	pub name: String,
	pub ty: Type,
	pub reason: ConstraintReason,
	// Back-edge to the AST site that emitted this constraint. Discharge /
	// generalization writes the resolved instance into this cell; codegen
	// reads it back via the AST node that shares the same cell.
	pub dispatch_cell: DispatchCell,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ClassConstraint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "class {} {}", self.name, self.ty)
	}
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ConstraintReason {
	pub range: Range,
}

pub fn eq_constraint(t1: Type, t2: Type) -> Constraint {
	Constraint::Eq(
		t1,
		t2,
		ConstraintReason {
			range: Range::collapsed(0, 0),
		},
	)
}

impl Constraint {
	pub fn at(self, range: Range) -> Self {
		match self {
			Constraint::Eq(t1, t2, _) => Constraint::Eq(t1, t2, ConstraintReason { range }),
			Constraint::Class(ClassConstraint {
				name,
				ty,
				dispatch_cell,
				..
			}) => Constraint::Class(ClassConstraint {
				name,
				ty,
				reason: ConstraintReason { range },
				dispatch_cell,
			}),
			_ => self,
		}
	}
}
