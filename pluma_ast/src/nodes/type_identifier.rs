use super::*;
use crate::common::*;
use crate::value_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeIdentifierNode {
  pub pos: Position,
  pub name: String,
  pub generics: Vec<TypeExprNode>,
  pub constraints: Option<Vec<TypeConstraint>>,
}
