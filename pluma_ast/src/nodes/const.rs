use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ConstNode {
  pub pos: Position,
  pub name: IdentifierNode,
  pub value: ExprNode,
}
