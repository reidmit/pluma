use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct TypeIdentifierNode {
  pub pos: Position,
  pub name: String,
  pub generics: Vec<TypeExprNode>,
}
