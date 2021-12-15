use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ModuleNode {
	pub pos: Position,
	pub body: Vec<StatementNode>,
}
