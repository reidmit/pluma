use crate::common::*;
use std::fmt;

#[derive(Clone)]
pub struct IdentifierNode {
  pub pos: Position,
  pub name: String,
}

#[cfg(debug_assertions)]
impl fmt::Debug for IdentifierNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Ident{:?} {:#?}", self.pos, self.name)
  }
}
