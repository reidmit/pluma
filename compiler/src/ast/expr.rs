use super::*;
use crate::location::*;
use crate::types::*;

pub struct ExprNode {
	pub ty: Type,
	pub kind: ExprKind,
	pub range: Range,
}

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

	/// e.g. `someTuple.0` or `(0, 1, 2).1`
	ElementAccess {
		receiver: Box<ExprNode>,
		index: usize,
	},

	/// e.g. `someRecord.field` or `{name: "reid"}.name`
	FieldAccess {
		receiver: Box<ExprNode>,
		field: IdentifierNode,
	},

	Fun(FunNode),
	Call(CallNode),
	EmptyTuple,
	Grouping(Box<ExprNode>),
	Identifier(IdentifierNode),
	Interpolation(Vec<ExprNode>),
	Let(LetNode),
	Literal(LiteralNode),
	Record(Vec<(IdentifierNode, ExprNode)>),
	Tuple(Vec<ExprNode>),
	Regex(RegexNode),

	// the below are not fully implemented yet!
	List(Vec<ExprNode>),
	If(IfNode),
	When(WhenNode),
	While(WhileNode),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("expr({:#?}) :: {}", self.range, self.ty))
			.field("kind", &self.kind)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ExprKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use ExprKind::*;

		match &self {
			BinaryOperation { op, left, right } => f
				.debug_struct(&format!("binary {:#?}", op))
				.field("left", left)
				.field("right", right)
				.finish(),

			UnaryOperation { op, right } => {
				write!(f, "unary {} {:#?}", op, right)
			}

			ElementAccess { receiver, index } => f
				.debug_struct("element-access")
				.field("receiver", receiver)
				.field("index", index)
				.finish(),

			FieldAccess { receiver, field } => f
				.debug_struct("field-access")
				.field("receiver", receiver)
				.field("field", field)
				.finish(),

			Fun(fun) => {
				write!(f, "{:#?}", fun)
			}

			Call(call) => {
				write!(f, "{:#?}", call)
			}

			EmptyTuple => {
				write!(f, "()")
			}

			Grouping(inner) => {
				write!(f, "{:#?}", inner)
			}

			Identifier(ident) => {
				write!(f, "{:#?}", ident)
			}

			If(if_node) => {
				write!(f, "{:#?}", if_node)
			}

			Interpolation(parts) => {
				write!(f, "interpolation {:#?}", parts)
			}

			Let(let_node) => {
				write!(f, "{:#?}", let_node)
			}

			List(elements) => {
				write!(f, "{:#?}", elements)
			}

			Literal(literal) => {
				write!(f, "{:#?}", literal)
			}

			Record(fields) => {
				write!(f, "record {:#?}", fields)
			}

			Regex(regex) => {
				write!(f, "{:#?}", regex)
			}

			Tuple(entries) => {
				write!(f, "tuple {:#?}", entries)
			}

			When(when_node) => {
				write!(f, "{:#?}", when_node)
			}

			While(while_node) => {
				write!(f, "{:#?}", while_node)
			}
		}
	}
}
