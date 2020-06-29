use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TopLevelStatementNode {
  pub pos: Position,
  pub kind: TopLevelStatementKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TopLevelStatementKind {
  Let(LetNode),
  Const(ConstNode),
  TypeDef(TypeDefNode),
  IntrinsicTypeDef(IntrinsicTypeDefNode),
  Def(DefNode),
  IntrinsicDef(IntrinsicDefNode),
  Expr(ExprNode),
  VisibilityMarker(ExportVisibility),
}
