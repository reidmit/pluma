use super::*;
use crate::expr_type::*;

pub struct LambdaNode {
	pub span: Span,
	pub params: Vec<LambdaParamNode>,
	pub body: Vec<ExprNode>,
}

pub struct LambdaParamNode {
	pub ident: IdentifierNode,
	pub inferred_type: ExprType,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LambdaNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("lambda({}-{})", self.span.0, self.span.1,))
			.field("params", &self.params)
			.field("body", &self.body)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LambdaParamNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:#?} :: {}", self.ident, self.inferred_type)
	}
}
