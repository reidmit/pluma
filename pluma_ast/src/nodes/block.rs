use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct BlockNode {
  pub pos: Position,
  pub params: Vec<IdentifierNode>,
  pub body: Vec<StatementNode>,
}
