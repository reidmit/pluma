use crate::expr_type::*;

pub type ConstraintSet = Vec<Constraint>;

pub enum Constraint {
  Eq(ExprType, ExprType),
}
