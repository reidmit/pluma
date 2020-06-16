use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct TopLevelStatementNode {
  pub pos: Position,
  pub kind: TopLevelStatementKind,
}

#[derive(Debug)]
pub enum TopLevelStatementKind {
  Let(LetNode),
  TypeDef(TypeDefNode),
  IntrinsicTypeDef(IntrinsicTypeDefNode),
  Def(DefNode),
  IntrinsicDef(IntrinsicDefNode),
  Expr(ExprNode),
  PrivateMarker,
}
