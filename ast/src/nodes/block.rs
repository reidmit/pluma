use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct BlockNode {
  pub pos: Position,
  pub param: Option<PatternNode>,
  pub body: Vec<StatementNode>,
}
