use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct IdentifierNode {
  pub pos: Position,
  pub name: String,
}
