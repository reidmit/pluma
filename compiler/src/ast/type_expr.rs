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

// Source-faithful rendering of a written type annotation. Unlike `Type`'s
// `Display` (which shows the *inferred*, alias-expanded type), this echoes the
// surface syntax the author wrote — so a `user-id` alias stays `user-id`
// rather than collapsing to its underlying `int`. Used for hover, where the
// name the author chose is more informative than its expansion.
impl std::fmt::Display for TypeExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.kind {
			TypeExprKind::Single(ident) => write!(f, "{}", ident),
			TypeExprKind::Func(params, ret) => {
				write!(f, "fun")?;
				for p in params {
					write!(f, " {}", p)?;
				}
				write!(f, " -> {}", ret)
			}
			TypeExprKind::Tuple(items) => {
				let parts: Vec<String> = items.iter().map(|t| t.to_string()).collect();
				write!(f, "({})", parts.join(", "))
			}
			TypeExprKind::Record(fields) => {
				if fields.is_empty() {
					return write!(f, "{{}}");
				}
				let parts: Vec<String> = fields
					.iter()
					.map(|(name, ty)| format!("{} :: {}", name.name, ty))
					.collect();
				write!(f, "{{{}}}", parts.join(", "))
			}
			TypeExprKind::EmptyTuple => write!(f, "()"),
			TypeExprKind::Grouping(inner) => write!(f, "({})", inner),
			TypeExprKind::Wildcard => write!(f, "_"),
		}
	}
}

impl std::fmt::Display for TypeIdentifierNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if let Some(m) = &self.module {
			write!(f, "{}.", m.name)?;
		}
		write!(f, "{}", self.name)?;
		for g in &self.generics {
			write!(f, " {}", g)?;
		}
		Ok(())
	}
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
