use super::*;
use crate::common::*;
use std::fmt;

pub struct StatementNode {
  pub pos: Position,
  pub kind: StatementKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum StatementKind {
  Let(LetNode),
  Expr(ExprNode),
}

#[cfg(debug_assertions)]
impl fmt::Debug for StatementNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Statement{:?} ", self.pos)?;

    match &self.kind {
      _ => write!(f, "{:#?}", self.kind),
    }
  }
}
