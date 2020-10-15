use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
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

  Concat,
}
