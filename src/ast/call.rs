use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CallNode {
	pub loc: Location,
	pub callee: Box<ExprNode>,
	pub args: Vec<ExprNode>,
}
