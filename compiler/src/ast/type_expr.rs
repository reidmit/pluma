use super::*;

pub struct TypeExprNode {
	pub span: Span,
	pub kind: TypeExprKind,
}

pub enum TypeExprKind {
	// e.g. string or dict<int, string>
	Single(TypeIdentifierNode),
	// e.g. fn string int -> bool
	Func(Vec<TypeExprNode>, Box<TypeExprNode>),
	// e.g. (string, bool)
	Tuple(Vec<TypeExprNode>),
	// e.g. {a: string, b: bool}
	Record(Vec<(IdentifierNode, TypeExprNode)>),
	// e.g. ()
	EmptyTuple,
	// e.g. (string) or (fn string -> bool)
	Grouping(Box<TypeExprNode>),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for TypeExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!(
			"type({}-{}) {:#?}",
			self.span.0, self.span.1, self.kind
		))
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
		}
	}
}
