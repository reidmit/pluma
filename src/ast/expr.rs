use super::*;
use crate::expr_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ExprNode {
	pub span: Span,
	pub kind: ExprKind,
	pub inferred_type: ExprType,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ExprKind {
	BinaryOperation {
		op: OperatorNode,
		left: Box<ExprNode>,
		right: Box<ExprNode>,
	},
	UnaryOperation {
		op: Operator,
		right: Box<ExprNode>,
	},
	Lambda(LambdaNode),
	Call(CallNode),
	EmptyTuple,
	Grouping(Box<ExprNode>),
	Identifier(IdentifierNode),
	If(IfNode),
	Interpolation(Vec<ExprNode>),
	Let(LetNode),
	List(Vec<ExprNode>),
	Literal(LiteralNode),
	Record(Vec<(IdentifierNode, ExprNode)>),
	Regex(RegexNode),
	Tuple(Vec<ExprNode>),
	When(WhenNode),
	While(WhileNode),
}
