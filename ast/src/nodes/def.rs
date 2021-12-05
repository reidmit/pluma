use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DefNode {
  pub pos: Position,
  pub has_receiver: bool,
  pub name_parts: Vec<IdentifierNode>,
  pub block: BlockNode,
}
