use crate::tokens::Token;
use std::fmt;

#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ParseError {
	pub pos: (usize, usize),
	pub kind: ParseErrorKind,
}

#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ParseErrorKind {
	EmptyRegularExpression,
	EmptyRegularExpressionGroup,
	EmptyRegularExpressionCount,
	InvalidBinaryDigit,
	InvalidDecimalDigit,
	InvalidHexDigit,
	InvalidOctalDigit,
	InvalidRegularExpressionCountModifier,
	MissingRightHandSideOfAssignment,
	MissingReturnType,
	OverflowingIntegerLiteral,
	UnclosedInterpolation,
	UnclosedString,
	UnexpectedEOF { expected: Token },
	UnexpectedToken { actual: Token, expected: Token },
	UnexpectedTokenExpectedEOF { actual: Token },
}

impl fmt::Display for ParseError {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		use ParseErrorKind::*;

		match self.kind {
			EmptyRegularExpression => write!(f, "Empty regular expression."),
			EmptyRegularExpressionCount => {
				write!(f, "Empty repetition count in regular expression.")
			}
			EmptyRegularExpressionGroup => write!(f, "Empty grouping in regular expression."),
			InvalidBinaryDigit => write!(f, "Invalid binary digits."),
			InvalidDecimalDigit => write!(f, "Invalid digits."),
			InvalidHexDigit => write!(f, "Invalid hex digits."),
			InvalidOctalDigit => write!(f, "Invalid octal digits."),
			InvalidRegularExpressionCountModifier => {
				write!(f, "Invalid regular expression count modifier.")
			}
			MissingRightHandSideOfAssignment => write!(f, "MissingRightHandSideOfAssignment"),
			MissingReturnType => write!(
				f,
				"Missing return type after '->' in function type expression"
			),
			OverflowingIntegerLiteral => write!(f, "OverflowingIntegerLiteral"),
			UnclosedInterpolation => write!(f, "Unclosed string interpolation."),
			UnclosedString => write!(f, "Unclosed string."),
			UnexpectedEOF { expected } => write!(f, "Unexpected end of file. Expected {}.", expected),
			UnexpectedToken { actual, expected } => {
				write!(f, "Unexpected token ({}). Expected {}.", actual, expected)
			}
			UnexpectedTokenExpectedEOF { actual } => {
				write!(f, "Unexpected token ({}). Expected end of file.", actual)
			}
		}
	}
}
