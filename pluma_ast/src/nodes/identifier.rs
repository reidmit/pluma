use crate::common::*;

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct IdentifierNode {
  pub pos: Position,
  pub name: String,
}
