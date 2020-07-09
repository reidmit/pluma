use super::*;
use crate::common::*;

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct UseNode {
  pub pos: Position,
  pub module_name: String,
  pub qualifier: Option<IdentifierNode>,
}
