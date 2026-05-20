use super::*;
use crate::location::Range;

#[derive(Clone)]
pub struct PatternNode {
	pub range: Range,
	pub kind: PatternKind,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum PatternKind {
	// e.g. if val is x { ... }
	Identifier(IdentifierNode),
	// e.g. if val is enum-variant a b { ... }
	Constructor(IdentifierNode, Vec<PatternNode>),
	// e.g. if val is (a, b) { ... }
	Tuple(Vec<PatternNode>),
	// e.g. if val is {a: 1, b: 2} { ... }
	// Without a `rest`, the subject must have exactly these fields. With
	// `rest = Some(_)`, the subject may carry extra fields (open match).
	Record {
		fields: Vec<(IdentifierNode, PatternNode)>,
		rest: Option<RecordRestPattern>,
	},
	// e.g. when items is [a, b, ...rest] { ... }
	// rest = None             — exact-length match (no `...`)
	// rest = Some(no binding) — anonymous `...`, no name capture
	// rest = Some(name)       — `...name`, binds remainder as `list a`
	List {
		items: Vec<PatternNode>,
		rest: Option<ListRestPattern>,
	},
	// e.g. if val is _ { ... }
	Underscore,
	// e.g. if val is 1 { ... }
	Literal(LiteralNode),
	// e.g. if name is "$(first) $(last)" { ... }
	Interpolation(Vec<ExprNode>),
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ListRestPattern {
	// Span of the `...` (or `...name`) for diagnostics.
	pub range: Range,
	// `Some` when written as `...name`; `None` for anonymous `...`.
	pub binding: Option<IdentifierNode>,
}

// Trailing rest in a record pattern. Parallels `ListRestPattern` — both
// `range` and `binding` mean the same thing. Named rest (`...name`)
// isn't analyzed/codegened yet; the field is here for parser symmetry
// and forward compatibility.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct RecordRestPattern {
	pub range: Range,
	pub binding: Option<IdentifierNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for PatternNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "pattern({:#?}) {:#?}", self.range, self.kind)
	}
}
