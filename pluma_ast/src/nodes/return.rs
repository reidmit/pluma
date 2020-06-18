use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ReturnNode {
  pub pos: Position,
  pub value: ExprNode,
}
