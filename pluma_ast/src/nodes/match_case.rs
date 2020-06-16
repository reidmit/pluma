use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct MatchCaseNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub body: ExprNode,
}
