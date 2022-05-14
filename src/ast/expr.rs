use super::*;
use crate::value_type::*;

pub struct ExprNode {
	pub pos: Position,
	pub kind: ExprKind,
	pub resolved_type: ValueType,
}

pub enum ExprKind {
	Access {
		receiver: Box<ExprNode>,
		property: Box<ExprNode>,
	},
	Assignment {
		left: Box<ExprNode>,
		right: Box<ExprNode>,
	},
	BinaryOperation {
		op: Operator,
		left: Box<ExprNode>,
		right: Box<ExprNode>,
	},
	UnaryOperation {
		op: Operator,
		right: Box<ExprNode>,
	},
	Lambda(LambdaNode),
	Call(CallNode),
	Dict(Vec<(ExprNode, ExprNode)>),
	EmptyTuple,
	For(ForNode),
	Grouping(Box<ExprNode>),
	Identifier(IdentifierNode),
	If(IfNode),
	Interpolation(Vec<ExprNode>),
	Let(LetNode),
	List(Vec<ExprNode>),
	Literal(LiteralNode),
	RegExpr(RegExprNode),
	Tuple(Vec<TupleEntry>),
	When(WhenNode),
	While(WhileNode),
}

pub struct TupleEntry(pub Option<IdentifierNode>, pub ExprNode);

impl std::fmt::Debug for ExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "expr:{}-{} ({:#?})", self.pos.0, self.pos.1, self.kind)
	}
}

impl std::fmt::Debug for ExprKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use ExprKind::*;

		match &self {
			Access { receiver, property } => write!(f, "{:?}.{:?}", receiver, property),
			Assignment { left, right } => write!(f, "({:?}) {:?}", left, right),
			BinaryOperation { op, left, right } => write!(f, "{:#?} {:#?}", op, vec![left, right]),
			UnaryOperation { op, right } => write!(f, "{:#?} {:#?}", op, right),
			Lambda(lambda) => write!(f, "{:#?}", lambda),
			Call(call) => write!(f, "{:#?}", call),
			Dict(entries) => write!(f, "{:#?}", entries),
			EmptyTuple => write!(f, "()"),
			For(for_node) => write!(f, "{:#?}", for_node),
			Grouping(expr) => write!(f, "grouping {:#?}", expr),
			Identifier(ident) => write!(f, "{:#?}", ident),
			If(if_node) => write!(f, "{:#?}", if_node),
			Interpolation(parts) => write!(f, "interpolation {:#?}", parts),
			Let(let_node) => write!(f, "{:#?}", let_node),
			List(elements) => write!(f, "{:#?}", elements),
			Literal(lit) => write!(f, "{:?}", lit),
			RegExpr(regex) => write!(f, "{:#?}", regex),
			Tuple(entries) => write!(f, "tuple {:#?}", entries),
			When(when_node) => write!(f, "{:#?}", when_node),
			While(while_node) => write!(f, "{:#?}", while_node),
		}
	}
}

impl std::fmt::Debug for TupleEntry {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if let Some(label) = &self.0 {
			write!(f, "(label {:?}) {:#?}", label, self.1)
		} else {
			write!(f, "{:#?}", self.1)
		}
	}
}
