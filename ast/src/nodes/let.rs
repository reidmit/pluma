use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct LetNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub value: ExprNode,
}
