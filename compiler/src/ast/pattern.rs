use super::*;
use crate::location::Range;

pub struct PatternNode {
	pub range: Range,
	pub kind: PatternKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum PatternKind {
	// e.g. if val is x { ... }
	Identifier(IdentifierNode),
	// e.g. if val is enum-variant a b { ... }
	Constructor(IdentifierNode, Vec<PatternNode>),
	// e.g. if val is (a, b) { ... }
	Tuple(Vec<PatternNode>),
	// e.g. if val is {a: 1, b: 2} { ... }
	Record(Vec<(IdentifierNode, PatternNode)>),
	// e.g. if val is _ { ... }
	Underscore,
	// e.g. if val is 1 { ... }
	Literal(LiteralNode),
	// e.g. if name is "$(first) $(last)" { ... }
	Interpolation(Vec<ExprNode>),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for PatternNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "pattern({:#?}) {:#?}", self.range, self.kind)
	}
}
