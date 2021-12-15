use super::*;
use crate::common::*;
use crate::value_type::ValueType;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeExprNode {
	pub pos: Position,
	pub kind: TypeExprKind,
	pub typ: ValueType,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeExprKind {
	// e.g. string or dict<int, string>
	Single(TypeIdentifierNode),
	// e.g. string -> bool
	Func(Box<TypeExprNode>, Box<TypeExprNode>),
	// e.g. (a: string, b: bool) or (string, bool)
	Tuple(Vec<(Option<IdentifierNode>, TypeExprNode)>),
	// e.g. ()
	EmptyTuple,
	// e.g. (string) or (string -> bool)
	Grouping(Box<TypeExprNode>),
}

impl TypeExprNode {
	pub fn to_type_identifier_mut(&mut self) -> &mut TypeIdentifierNode {
		match &mut self.kind {
			TypeExprKind::Single(ident) => ident,
			_ => unreachable!("must be called on type identifier"),
		}
	}
}
