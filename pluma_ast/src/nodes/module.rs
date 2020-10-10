use super::*;
use crate::common::*;
use std::fmt;

pub struct ModuleNode {
  pub pos: Position,
  pub body: Vec<TopLevelStatementNode>,
}

#[cfg(debug_assertions)]
impl fmt::Debug for ModuleNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Module{:?} {:#?}", self.pos, self.body)
  }
}
