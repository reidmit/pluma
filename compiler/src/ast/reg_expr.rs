use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct RegexNode {
	pub span: Span,
	pub kind: RegexKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
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
