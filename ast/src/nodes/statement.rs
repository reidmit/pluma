use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct StatementNode {
  pub pos: Position,
  pub kind: StatementKind,
}

#[derive(Debug)]
pub enum StatementKind {
  Let(LetNode),
  Expr(ExprNode),
  Return(ReturnNode),
}
