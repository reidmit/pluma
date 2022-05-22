use super::*;

pub struct CallNode {
	pub span: Span,
	pub callee: Box<ExprNode>,
	pub args: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for CallNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("call({}-{})", self.span.0, self.span.1))
			.field("callee", &self.callee)
			.field("args", &self.args)
			.finish()
	}
}
