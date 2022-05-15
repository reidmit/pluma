use super::*;

pub struct RegexNode {
	pub pos: Position,
	pub kind: RegexKind,
}

pub enum RegexKind {
	Literal(String),
	CharacterClass(String),
	OneOrMore(Box<RegexNode>),
	ZeroOrMore(Box<RegexNode>),
	OneOrZero(Box<RegexNode>),
	AtLeastCount(Box<RegexNode>, usize),
	AtMostCount(Box<RegexNode>, usize),
	ExactCount(Box<RegexNode>, usize),
	RangeCount(Box<RegexNode>, usize, usize),
	Grouping(Box<RegexNode>),
	Sequence(Vec<RegexNode>),
	Alternation(Vec<RegexNode>),
	NamedCapture(String, Box<RegexNode>),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for RegexNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "regex:{}-{} {:#?}", self.pos.0, self.pos.1, self.kind)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for RegexKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use RegexKind::*;

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
