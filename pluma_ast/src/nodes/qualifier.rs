use crate::common::*;
use std::fmt;

#[derive(Clone)]
pub struct QualifierNode {
  pub pos: Position,
  pub name: String,
}

#[cfg(debug_assertions)]
impl fmt::Debug for QualifierNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Qualifier{:?} {:#?}", self.pos, self.name)
  }
}
