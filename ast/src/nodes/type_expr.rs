use super::*;
use crate::common::*;
use crate::value_type::ValueType;

#[derive(Debug)]
pub struct TypeExprNode {
  pub pos: Position,
  pub kind: TypeExprKind,
  pub typ: ValueType,
}

#[derive(Debug)]
pub enum TypeExprKind {
  // e.g. String or Dict<Int, String>
  Single(TypeIdentifierNode),
  // e.g. String -> Bool
  Func(Box<TypeExprNode>, Box<TypeExprNode>),
  // e.g. (String, Bool)
  Tuple(Vec<TypeExprNode>),
  // e.g. ()
  EmptyTuple,
  // e.g. (String) or (String -> Bool)
  Grouping(Box<TypeExprNode>),
}
