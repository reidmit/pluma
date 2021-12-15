use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct MatchNode {
	pub pos: Position,
	pub subject: Box<ExprNode>,
	pub cases: Vec<MatchCaseNode>,
}
