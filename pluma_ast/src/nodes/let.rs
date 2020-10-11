use super::*;
use crate::common::*;
use std::fmt;

pub struct LetNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub value: ExprNode,
}

#[cfg(debug_assertions)]
impl fmt::Debug for LetNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Let{:?} {:#?} {:#?}", self.pos, self.pattern, self.value)
  }
}
