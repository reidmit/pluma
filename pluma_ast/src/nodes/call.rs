use super::*;
use crate::common::*;
use crate::value_type::ValueType;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CallNode {
  pub pos: Position,
  pub callee: Box<ExprNode>,
  pub args: Vec<ExprNode>,
  pub typ: ValueType,
}
