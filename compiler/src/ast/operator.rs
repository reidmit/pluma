use crate::{location::Range, tokens::Token};

#[derive(Clone)]
pub struct OperatorNode {
	pub range: Range,
	pub kind: Operator,
}

#[derive(Clone)]
pub enum Operator {
	Addition,
	Chain,
	Concat,
	Division,
	Equality,
	Exponentiation,
	FieldAccess,
	FunctionCall,
	GreaterThan,
	GreaterThanEquals,
	IndexAccess,
	Inequality,
	LessThan,
	LessThanEquals,
	LogicalAnd,
	LogicalNot,
	LogicalOr,
	Multiplication,
	NullCoalescing,
	Range,
	Remainder,
	SubtractionOrNegation,
}

impl Operator {
	pub fn from_token(token: Token) -> Option<Operator> {
		match token {
			Token::BangEqual(..) => Some(Operator::Inequality),
			Token::Dot(..) => Some(Operator::FieldAccess),
			Token::DoubleAnd(..) => Some(Operator::LogicalAnd),
			Token::DoubleDot(..) => Some(Operator::Range),
			Token::DoubleEqual(..) => Some(Operator::Equality),
			Token::DoublePipe(..) => Some(Operator::LogicalOr),
			Token::DoublePlus(..) => Some(Operator::Concat),
			Token::DoubleQuestion(..) => Some(Operator::NullCoalescing),
			Token::DoubleStar(..) => Some(Operator::Exponentiation),
			Token::ForwardSlash(..) => Some(Operator::Division),
			Token::LeftAngle(..) => Some(Operator::LessThan),
			Token::LeftAngleEqual(..) => Some(Operator::LessThanEquals),
			// `[` is intentionally not parsed as an infix operator: `f [x]`
			// reads as a function call with a list-literal argument. If/when
			// real indexing comes back, design it with explicit syntax (e.g.
			// no whitespace between subject and `[`).
			Token::Minus(..) => Some(Operator::SubtractionOrNegation),
			Token::Percent(..) => Some(Operator::Remainder),
			Token::Pipe(..) => Some(Operator::Chain),
			Token::Plus(..) => Some(Operator::Addition),
			Token::RightAngle(..) => Some(Operator::GreaterThan),
			Token::RightAngleEqual(..) => Some(Operator::GreaterThanEquals),
			Token::Star(..) => Some(Operator::Multiplication),
			_ => None,
		}
	}

	pub fn infix_binding_power(&self) -> Option<(u8, u8)> {
		use Operator::*;

		// if left < right, it's left-associative
		// if left > right, it's right-associative
		// lower numbers bind weaker than higher numbers
		match &self {
			Chain => Some((0, 1)),
			Range => Some((10, 11)),
			LogicalOr | NullCoalescing => Some((20, 21)),
			LogicalAnd => Some((30, 31)),
			Equality | Inequality => Some((40, 41)),
			LessThan | LessThanEquals | GreaterThan | GreaterThanEquals => Some((50, 51)),
			Addition | SubtractionOrNegation => Some((60, 61)),
			// `++` (string concat) binds like addition: left-associative and
			// tighter than comparisons, so `a ++ b == c ++ d` groups as
			// `(a ++ b) == (c ++ d)`.
			Concat => Some((60, 61)),
			Multiplication | Division | Remainder => Some((70, 71)),
			Exponentiation => Some((81, 80)),
			FunctionCall => Some((90, 91)),
			FieldAccess | IndexAccess => Some((100, 101)),
			_ => None,
		}
	}

	pub fn prefix_binding_power(&self) -> ((), u8) {
		use Operator::*;

		// these numbers are relative to those above (see infix_binding_power);
		match &self {
			SubtractionOrNegation => ((), 75),
			LogicalNot => ((), 35),
			_ => unreachable!(),
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for OperatorNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "op({:#?}) `{}`", self.range, self.kind)
	}
}

impl std::fmt::Display for Operator {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use Operator::*;

		match &self {
			Addition => write!(f, "+"),
			Chain => write!(f, "|"),
			Concat => write!(f, "++"),
			Division => write!(f, "/"),
			Equality => write!(f, "=="),
			Exponentiation => write!(f, "**"),
			FieldAccess => write!(f, "."),
			FunctionCall => write!(f, "call"),
			GreaterThan => write!(f, ">"),
			GreaterThanEquals => write!(f, ">="),
			IndexAccess => write!(f, "[]"),
			Inequality => write!(f, "!="),
			LessThan => write!(f, "<"),
			LessThanEquals => write!(f, "<="),
			LogicalAnd => write!(f, "&&"),
			LogicalNot => write!(f, "!"),
			LogicalOr => write!(f, "||"),
			Multiplication => write!(f, "*"),
			NullCoalescing => write!(f, "??"),
			Range => write!(f, ".."),
			Remainder => write!(f, "%"),
			SubtractionOrNegation => write!(f, "-"),
		}
	}
}
