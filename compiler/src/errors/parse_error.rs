use crate::{location::Range, tokens::Token};
use std::fmt;

#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ParseError {
	pub range: Range,
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
	QuantifierOnRegexAnchor,
	InvalidExpressionAfterDot,
	InvalidDefBody,
	MissingReturnType,
	OverflowingIntegerLiteral,
	UnclosedInterpolation,
	UnclosedString,
	InvalidBytesEscape,
	InvalidHexEscape,
	BuiltinExpectsPlainString,
	ExpectedExpressionAfterSpread,
	UnexpectedEOF { expected: Token },
	UnexpectedToken { actual: Token, expected: Token },
	UnexpectedTopLevelToken { actual: Token },
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
			QuantifierOnRegexAnchor => write!(
				f,
				"A quantifier cannot be applied to an anchor (`^`, `$`, or `%`)."
			),
			InvalidExpressionAfterDot => write!(
				f,
				"Invalid expression after `.`: expected either an integer or a field name."
			),
			InvalidDefBody => write!(
				f,
				"Invalid body in `def` statement: expected either an expression or a type."
			),
			MissingReturnType => write!(
				f,
				"Missing return type after '->' in function type expression"
			),
			OverflowingIntegerLiteral => write!(f, "Overflowing integer literal."),
			UnclosedInterpolation => write!(f, "Unclosed string interpolation."),
			UnclosedString => write!(f, "Unclosed string."),
			InvalidBytesEscape => write!(
				f,
				"Invalid escape sequence in bytes literal. Valid escapes are \\\\, \\', \\0, \\t, \\r, \\n, and \\xNN."
			),
			InvalidHexEscape => write!(
				f,
				"Invalid \\x escape in bytes literal: expected two hex digits."
			),
			BuiltinExpectsPlainString => write!(
				f,
				"`built-in` requires a plain string literal naming the builtin tag."
			),
			ExpectedExpressionAfterSpread => write!(
				f,
				"Expected an expression after `...` in a list literal."
			),
			UnexpectedEOF { expected } => write!(f, "Unexpected end of file. Expected {}.", expected),
			UnexpectedToken { actual, expected } => {
				write!(f, "Unexpected token ({}). Expected {}.", actual, expected)
			}
			UnexpectedTopLevelToken { actual } => {
				write!(
					f,
					"Unexpected token ({}). Expected a top-level definition (`def`, `enum`, `alias`, `trait`, `test`, or `implement`).",
					actual
				)
			}
		}
	}
}
