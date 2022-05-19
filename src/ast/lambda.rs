use super::*;
use crate::expr_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct LambdaNode {
	pub span: Span,
	pub params: Vec<LambdaParamNode>,
	pub body: Vec<ExprNode>,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct LambdaParamNode {
	pub ident: IdentifierNode,
	pub inferred_type: ExprType,
}
