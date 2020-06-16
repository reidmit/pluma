use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct MatchNode {
  pub pos: Position,
  pub subject: Box<ExprNode>,
  pub cases: Vec<MatchCaseNode>,
}
