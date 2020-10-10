use super::*;
use crate::common::*;
use std::fmt;

pub struct TopLevelStatementNode {
  pub pos: Position,
  pub kind: TopLevelStatementKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TopLevelStatementKind {
  Let(LetNode),
  TypeDef(TypeDefNode),
  IntrinsicTypeDef(IntrinsicTypeDefNode),
  Def(DefNode),
  IntrinsicDef(IntrinsicDefNode),
  Expr(ExprNode),
  VisibilityMarker(ExportVisibility),
}

#[cfg(debug_assertions)]
impl fmt::Debug for TopLevelStatementNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "TopLevelStatement{:?} ", self.pos)?;

    match &self.kind {
      _ => write!(f, "{:#?}", self.kind),
    }
  }
}
