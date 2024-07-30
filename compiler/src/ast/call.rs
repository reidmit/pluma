use crate::location::Range;

use super::*;

pub struct CallNode {
	pub range: Range,
	pub callee: Box<ExprNode>,
	pub args: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for CallNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("call({:#?})", self.range))
			.field("callee", &self.callee)
			.field("args", &self.args)
			.finish()
	}
}
