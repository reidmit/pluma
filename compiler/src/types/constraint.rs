use crate::{location::Range, types::*};

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Constraint {
	Eq(Type, Type, ConstraintReason),
	Gen(Scheme, Type),
	Inst(usize, Type),
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
			_ => self,
		}
	}
}
