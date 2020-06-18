use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct OperatorNode {
  pub pos: Position,
  pub name: String,
}
