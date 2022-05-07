use super::*;

pub struct ExprNode {
	pub pos: Position,
	pub kind: ExprKind,
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
	Lambda(LambdaNode),
	Call(CallNode),
	Dict {
		entries: Vec<(ExprNode, ExprNode)>,
	},
	EmptyTuple,
	Grouping(Box<ExprNode>),
	Identifier(IdentifierNode),
	Interpolation {
		parts: Vec<ExprNode>,
	},
	Let(LetNode),
	List {
		elements: Vec<ExprNode>,
	},
	Literal(LiteralNode),
	RegExpr(RegExprNode),
	Tuple {
		entries: Vec<(Option<IdentifierNode>, ExprNode)>,
	},
}

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
			Lambda(lambda) => write!(f, "{:#?}", lambda),
			Call(call) => write!(f, "{:#?}", call),
			Dict { entries } => write!(f, "{:#?}", entries),
			EmptyTuple => write!(f, "()"),
			Grouping(expr) => write!(f, "({:#?})", expr),
			Identifier(ident) => write!(f, "{:#?}", ident),
			Interpolation { parts } => write!(f, "interpolation {:#?}", parts),
			Let(let_node) => write!(f, "{:#?}", let_node),
			List { elements } => write!(f, "{:#?}", elements),
			Literal(lit) => write!(f, "{:?}", lit),
			RegExpr(regex) => write!(f, "{:#?}", regex),
			Tuple { entries } => write!(f, "tuple {:#?}", entries),
		}
	}
}
