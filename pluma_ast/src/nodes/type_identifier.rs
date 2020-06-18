use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeIdentifierNode {
  pub pos: Position,
  pub name: String,
  pub generics: Vec<TypeExprNode>,
}
