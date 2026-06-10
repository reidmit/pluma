use crate::{location::Range, tokens::Token};

#[derive(Clone)]
pub struct OperatorNode {
	pub range: Range,
	pub kind: Operator,
}

#[derive(Clone)]
pub enum Operator {
	Addition,
	BitAnd,
	BitNot,
	BitOr,
	BitXor,
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
	ShiftLeft,
	ShiftRight,
	ShiftRightUnsigned,
	SubtractionOrNegation,
}

impl Operator {
	pub fn from_token(token: Token) -> Option<Operator> {
		match token {
			// Bare `&` is bitwise AND (infix). `&&` is `DoubleAnd` (logical and).
			Token::And(..) => Some(Operator::BitAnd),
			Token::BangEqual(..) => Some(Operator::Inequality),
			// `^` is bitwise XOR in expression position (it also anchors regex
			// literals, parsed in a separate context).
			Token::Caret(..) => Some(Operator::BitXor),
			Token::Dot(..) => Some(Operator::FieldAccess),
			Token::DoubleAnd(..) => Some(Operator::LogicalAnd),
			Token::DoubleDot(..) => Some(Operator::Range),
			Token::DoubleEqual(..) => Some(Operator::Equality),
			// `<<`/`>>`/`>>>` are the bit-shift operators.
			Token::DoubleLeftAngle(..) => Some(Operator::ShiftLeft),
			Token::DoublePipe(..) => Some(Operator::LogicalOr),
			Token::DoublePlus(..) => Some(Operator::Concat),
			Token::DoubleQuestion(..) => Some(Operator::NullCoalescing),
			Token::DoubleRightAngle(..) => Some(Operator::ShiftRight),
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
			// Bare `|` is bitwise OR. The pipe/chain operator is `|>` (`PipeArrow`).
			// Inside backtick literals `|` still means regex alternation, parsed
			// in a separate context.
			Token::Pipe(..) => Some(Operator::BitOr),
			Token::PipeArrow(..) => Some(Operator::Chain),
			Token::TripleRightAngle(..) => Some(Operator::ShiftRightUnsigned),
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
			LogicalOr => Some((20, 21)),
			// `??` is right-associative so `a ?? b ?? c` groups as
			// `a ?? (b ?? c)` — the only grouping that type-checks when each
			// `??` unwraps its left operand to a bare value.
			NullCoalescing => Some((21, 20)),
			LogicalAnd => Some((30, 31)),
			Equality | Inequality => Some((40, 41)),
			LessThan | LessThanEquals | GreaterThan | GreaterThanEquals => Some((50, 51)),
			// Bitwise operators bind tighter than comparison (so `x & m == 0` is
			// `(x & m) == 0`, not C's footgun) but looser than `+`/`-`. Among
			// themselves: shifts tightest, then `&`, `^`, `|` — matching the C
			// family's relative order.
			BitOr => Some((52, 53)),
			BitXor => Some((54, 55)),
			BitAnd => Some((56, 57)),
			ShiftLeft | ShiftRight | ShiftRightUnsigned => Some((58, 59)),
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
			BitNot => ((), 75),
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
			BitAnd => write!(f, "&"),
			BitNot => write!(f, "~"),
			BitOr => write!(f, "|"),
			BitXor => write!(f, "^"),
			Chain => write!(f, "|>"),
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
			ShiftLeft => write!(f, "<<"),
			ShiftRight => write!(f, ">>"),
			ShiftRightUnsigned => write!(f, ">>>"),
			SubtractionOrNegation => write!(f, "-"),
		}
	}
}
