use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct ReturnNode {
  pub pos: Position,
  pub value: ExprNode,
}
