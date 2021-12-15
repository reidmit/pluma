use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct UseNode {
	pub pos: Position,
	pub module_name: String,
	pub qualifier: Option<QualifierNode>,
}
