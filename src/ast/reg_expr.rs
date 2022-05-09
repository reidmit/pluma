use super::*;

pub struct RegExprNode {
	pub pos: Position,
	pub kind: RegExprKind,
}

pub enum RegExprKind {
	Literal(String),
	CharacterClass(String),
	OneOrMore(Box<RegExprNode>),
	ZeroOrMore(Box<RegExprNode>),
	OneOrZero(Box<RegExprNode>),
	AtLeastCount(Box<RegExprNode>, usize),
	AtMostCount(Box<RegExprNode>, usize),
	ExactCount(Box<RegExprNode>, usize),
	RangeCount(Box<RegExprNode>, usize, usize),
	Grouping(Box<RegExprNode>),
	Sequence(Vec<RegExprNode>),
	Alternation(Vec<RegExprNode>),
	NamedCapture(String, Box<RegExprNode>),
}

impl std::fmt::Debug for RegExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "regex:{}-{} {:#?}", self.pos.0, self.pos.1, self.kind)
	}
}

impl std::fmt::Debug for RegExprKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use RegExprKind::*;

		match &self {
			Literal(lit) => {
				write!(f, "literal {:?}", lit)
			}
			CharacterClass(class) => {
				write!(f, "char-class {}", class)
			}
			OneOrMore(node) => {
				write!(f, "one-or-more {:#?}", node)
			}
			ZeroOrMore(node) => {
				write!(f, "zero-or-more {:#?}", node)
			}
			OneOrZero(node) => {
				write!(f, "one-or-zero {:#?}", node)
			}
			AtLeastCount(node, count) => {
				write!(f, "at-least {} {:#?}", count, node)
			}
			AtMostCount(node, count) => {
				write!(f, "at-most {} {:#?}", count, node)
			}
			ExactCount(node, count) => {
				write!(f, "exactly {} {:#?}", count, node)
			}
			RangeCount(node, min, max) => {
				write!(f, "between {}-{} {:#?}", min, max, node)
			}
			Grouping(node) => {
				write!(f, "grouping {:#?}", node)
			}
			Sequence(nodes) => {
				write!(f, "sequence {:#?}", nodes)
			}
			Alternation(nodes) => {
				write!(f, "alternation {:#?}", nodes)
			}
			NamedCapture(label, node) => {
				write!(f, "capture `{}` {:#?}", label, node)
			}
		}
	}
}
