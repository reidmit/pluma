use super::*;

pub struct WhenNode {
	pub span: Span,
	pub subject: Box<ExprNode>,
	pub cases: Vec<CaseNode>,
}

pub struct CaseNode {
	pub span: Span,
	pub pattern: PatternNode,
	pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for WhenNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "when({}-{}) {:#?}", self.span.0, self.span.1, self.cases)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for CaseNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"case({}-{}) {:#?} {:#?}",
			self.span.0, self.span.1, self.pattern, self.body
		)
	}
}
