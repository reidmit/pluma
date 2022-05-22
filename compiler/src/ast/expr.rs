use super::*;
use crate::types::*;

pub struct ExprNode {
	pub span: Span,
	pub kind: ExprKind,
	pub ty: Type,
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

#[cfg(debug_assertions)]
impl std::fmt::Debug for ExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!(
			"expr({}-{}) :: {}",
			self.span.0, self.span.1, self.ty
		))
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
			Lambda(lambda) => {
				write!(f, "{:#?}", lambda)
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
				write!(f, "{:#?}", parts)
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
