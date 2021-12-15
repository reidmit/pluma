use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct LetNode {
	pub pos: Position,
	pub pattern: PatternNode,
	pub value: ExprNode,
}
