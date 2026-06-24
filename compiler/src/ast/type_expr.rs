use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct TypeExprNode {
	pub range: Range,
	pub kind: TypeExprKind,
}

#[derive(Clone)]
pub enum TypeExprKind {
	// e.g. string or dict<int, string>
	Single(TypeIdentifierNode),
	// e.g. fun string int -> bool
	Func(Vec<TypeExprNode>, Box<TypeExprNode>),
	// e.g. (string, bool)
	Tuple(Vec<TypeExprNode>),
	// e.g. {a: string, b: bool}
	Record(Vec<(IdentifierNode, TypeExprNode)>),
	// e.g. ()
	EmptyTuple,
	// e.g. (string) or (fun string -> bool)
	Grouping(Box<TypeExprNode>),
	// `_` — a deliberately-anonymous type argument. Resolves to a fresh
	// inference variable, exactly like a one-off named type var would, but
	// without the reader having to invent a name for a param they don't care
	// about. Required wherever a type constructor's argument is left to
	// inference: `task a _`, `dict _ string`.
	Wildcard,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TypeExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("type({:#?}) {:#?}", self.range, self.kind))
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TypeExprKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use TypeExprKind::*;

		match &self {
			Single(ident) => {
				write!(f, "{:#?}", ident)
			}

			Func(param_types, return_type) => {
				write!(f, "fun-type {:#?} -> {:#?}", param_types, return_type)
			}

			Tuple(entries) => {
				write!(f, "tuple-type {:#?}", entries)
			}

			Record(fields) => {
				write!(f, "record-type {:#?}", fields)
			}

			EmptyTuple => {
				write!(f, "empty-type ()")
			}

			Grouping(inner) => {
				write!(f, "{:#?}", inner)
			}

			Wildcard => {
				write!(f, "wildcard-type _")
			}
		}
	}
}
