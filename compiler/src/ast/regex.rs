use crate::location::Range;

#[derive(Clone)]
pub struct RegexNode {
	pub range: Range,
	pub kind: RegexKind,
}

#[derive(Clone)]
pub enum RegexKind {
	Literal(String),
	CharacterClass(String),
	Anchor(RegexAnchor),
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

#[derive(Copy, Clone)]
pub enum RegexAnchor {
	Start,
	End,
	Boundary,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for RegexNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("regex({:#?}) {:#?}", self.range, self.kind))
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for RegexKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use RegexKind::*;

		match &self {
			Literal(lit) => write!(f, "literal {:?}", lit),

			CharacterClass(cls) => write!(f, "class {}", cls),

			Anchor(a) => write!(f, "anchor {}", match a {
				RegexAnchor::Start => "start",
				RegexAnchor::End => "end",
				RegexAnchor::Boundary => "boundary",
			}),

			OneOrMore(inner) => write!(f, "one-or-more ({:#?})", inner),

			ZeroOrMore(inner) => write!(f, "zero-or-more ({:#?})", inner),

			OneOrZero(inner) => write!(f, "one-or-zero ({:#?})", inner),

			AtLeastCount(inner, count) => write!(f, "at-least {} ({:#?})", count, inner),

			AtMostCount(inner, count) => write!(f, "at-most {} ({:#?})", count, inner),

			ExactCount(inner, count) => write!(f, "exactly {} ({:#?})", count, inner),

			RangeCount(inner, min, max) => write!(f, "between {} and {} ({:#?})", min, max, inner),

			Grouping(inner) => {
				write!(f, "{:#?}", inner)
			}

			Sequence(inners) => write!(f, "sequence {:#?}", inners),

			Alternation(inners) => write!(f, "alternation {:#?}", inners),

			NamedCapture(name, inner) => write!(f, "capture {} {:#?}", name, inner),
		}
	}
}
