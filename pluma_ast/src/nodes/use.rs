use super::*;
use crate::common::*;

#[derive(Debug, Clone)]
pub struct UseNode {
  pub pos: Position,
  pub module_name: String,
  pub qualifier: Box<IdentifierNode>,
}
