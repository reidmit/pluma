use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct StatementNode {
  pub pos: Position,
  pub kind: StatementKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum StatementKind {
  Let(LetNode),
  Def(DefNode),
  Type(TypeDefNode),
  Expr(ExprNode),
}
