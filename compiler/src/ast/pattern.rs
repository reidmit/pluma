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
	// A variant match, e.g. `if val is color.red { ... }` (nullary) or
	// `is color.some x { ... }` (with payload). The head names the variant,
	// qualified by its enum (`enum.variant`) and, for an imported enum, its
	// module (`module.enum.variant`); a bare head (`variant`) is reserved for
	// prelude variants. `args` is empty for a nullary variant written
	// qualified.
	Constructor(ConstructorHead, Vec<PatternNode>),
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

// The head of a `Constructor` pattern: the variant name, optionally qualified.
// Three shapes, distinguished by how the source dotted the path:
//   `variant`               — bare: module = None, enum_name = None (prelude only)
//   `enum.variant`          — module = None, enum_name = Some
//   `module.enum.variant`   — module = Some, enum_name = Some (imported enum)
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ConstructorHead {
	pub range: Range,
	pub module: Option<IdentifierNode>,
	pub enum_name: Option<IdentifierNode>,
	pub variant: IdentifierNode,
}

impl ConstructorHead {
	// A bare head carries only the variant name — no enum/module qualifier.
	// These are reserved for prelude variants (`some`, `none`, `ok`, ...).
	pub fn is_bare(&self) -> bool {
		self.enum_name.is_none()
	}

	// The source form, re-joined with dots: `variant`, `enum.variant`, or
	// `module.enum.variant`. Used by the formatter and diagnostics.
	pub fn dotted(&self) -> String {
		let mut parts: Vec<&str> = Vec::with_capacity(3);
		if let Some(m) = &self.module {
			parts.push(&m.name);
		}
		if let Some(e) = &self.enum_name {
			parts.push(&e.name);
		}
		parts.push(&self.variant.name);
		parts.join(".")
	}
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
