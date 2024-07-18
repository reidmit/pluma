use super::*;
use crate::types::*;

pub struct FunNode {
	pub span: Span,
	pub params: Vec<FunParamNode>,
	pub body: Vec<ExprNode>,
}

pub struct FunParamNode {
	pub ident: IdentifierNode,
	pub ty: Type,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for FunNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("fun({}-{})", self.span.0, self.span.1,))
			.field("params", &self.params)
			.field("body", &self.body)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for FunParamNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:#?} :: {}", self.ident, self.ty)
	}
}
