use crate::common::*;
use std::fmt;

pub struct OperatorNode {
  pub pos: Position,
  pub kind: OperatorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum OperatorKind {
  Add,
  Subtract,

  Multiply,
  Divide,
  Mod,
  Exponent,

  BitwiseAnd,
  BitwiseOr,
  BitwiseXor,
  BitwiseNot,
  BitwiseLeftShift,
  BitwiseRightShift,

  LogicalAnd,
  LogicalOr,

  LessThan,
  GreaterThan,
  LessThanEquals,
  GreaterThanEquals,
  Equals,
  NotEquals,
}

#[cfg(debug_assertions)]
impl fmt::Debug for OperatorNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Operator{:?} {:?}", self.pos, self.kind)
  }
}
