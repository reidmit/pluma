use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct EnumVariantNode {
  pub pos: Position,
  pub kind: EnumVariantKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum EnumVariantKind {
  Identifier(IdentifierNode),
  Constructor(IdentifierNode, TypeExprNode),
}
