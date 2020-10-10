use super::*;
use crate::common::*;
use std::fmt;

pub struct BlockNode {
  pub pos: Position,
  pub params: Vec<PatternNode>,
  pub body: Vec<StatementNode>,
}

#[cfg(debug_assertions)]
impl fmt::Debug for BlockNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "Block{:?} ({:#?}) {:#?}",
      self.pos, self.params, self.body
    )
  }
}
