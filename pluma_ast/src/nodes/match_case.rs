use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct MatchCaseNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub body: ExprNode,
}
