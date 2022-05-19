use super::*;
use crate::expr_type::*;

pub struct LambdaNode {
	pub pos: Position,
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
		write!(
			f,
			"lambda:{}:{} {:#?} {:#?}",
			self.pos.0, self.pos.1, self.params, self.body
		)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LambdaParamNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}::{}", self.ident.name, self.inferred_type)
	}
}
