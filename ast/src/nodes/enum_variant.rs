use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct EnumVariantNode {
  pub pos: Position,
  pub kind: EnumVariantKind,
}

#[derive(Debug)]
pub enum EnumVariantKind {
  Identifier(IdentifierNode),
  Constructor(IdentifierNode, TypeExprNode),
}
