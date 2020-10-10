use super::*;
use crate::common::*;
use std::fmt;

#[derive(Clone)]
pub struct UseNode {
  pub pos: Position,
  pub module_name: String,
  pub qualifier: Option<QualifierNode>,
}

#[cfg(debug_assertions)]
impl fmt::Debug for UseNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "Use{:?} {:?} {:?}",
      self.pos, self.qualifier, self.module_name
    )
  }
}
