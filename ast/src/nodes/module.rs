use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct ModuleNode {
  pub pos: Position,
  pub body: Vec<TopLevelStatementNode>,
}
