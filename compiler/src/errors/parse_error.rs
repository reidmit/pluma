use crate::diagnostic::Reportable;
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
	LeadingDotOutsideUsing,
	InvalidDefBody,
	MissingReturnType,
	OverflowingIntegerLiteral,
	InvalidDurationUnit,
	DurationUnitsOutOfOrder,
	OverflowingDurationLiteral,
	UnclosedInterpolation,
	UnclosedString,
	InvalidBytesEscape,
	InvalidHexEscape,
	BuiltinExpectsPlainString,
	ExpectedExpressionAfterSpread,
	ExpectedExpressionAfterDefer,
	MisplacedRecordSpread,
	UnexpectedEOF { expected: Token },
	UnexpectedToken { actual: Token, expected: Token },
	UnexpectedTopLevelToken { actual: Token },
	MisplacedVisibility { keyword: &'static str },
	MisplacedRemote,
	// An expression was required here (a def body, or the operand after a
	// prefix/infix operator) but the next token can't start one. `found` is
	// the offending token, or `None` at end of file.
	ExpectedExpression { found: Option<Token> },
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
			LeadingDotOutsideUsing => write!(
				f,
				"A leading `.member` is only valid inside a `using` block."
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
			InvalidDurationUnit => write!(
				f,
				"Invalid duration literal. Expected `<amount><unit>` segments using the units d, h, m, s, ms, us, ns (e.g. 2m20s)."
			),
			DurationUnitsOutOfOrder => write!(
				f,
				"Duration units must each appear at most once, in descending order: d, h, m, s, ms, us, ns."
			),
			OverflowingDurationLiteral => write!(f, "Overflowing duration literal."),
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
			ExpectedExpressionAfterSpread => write!(f, "Expected an expression after `...`."),
			ExpectedExpressionAfterDefer => {
				write!(f, "Expected an expression after `defer`.")
			}
			MisplacedRecordSpread => write!(
				f,
				"A record update allows a single `...spread`, and it must come first (`{{ ...base, field: value }}`)."
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
			MisplacedVisibility { keyword } => {
				if keyword == "opaque" {
					write!(f, "`opaque` can only modify a top-level `enum`.")
				} else {
					write!(
						f,
						"`public` can only modify a top-level `def`, `enum`, or `alias`."
					)
				}
			}
			MisplacedRemote => {
				write!(f, "`remote` can only modify a top-level `def`.")
			}
			ExpectedExpression { found } => match found {
				Some(token) => write!(f, "Expected an expression, but found {}.", token),
				None => write!(
					f,
					"Expected an expression, but reached the end of the file."
				),
			},
		}
	}
}

impl Reportable for ParseError {
	fn code(&self) -> &'static str {
		use ParseErrorKind::*;
		match self.kind {
			EmptyRegularExpression => "E0001",
			EmptyRegularExpressionGroup => "E0002",
			EmptyRegularExpressionCount => "E0003",
			InvalidBinaryDigit => "E0004",
			InvalidDecimalDigit => "E0005",
			InvalidHexDigit => "E0006",
			InvalidOctalDigit => "E0007",
			InvalidRegularExpressionCountModifier => "E0008",
			QuantifierOnRegexAnchor => "E0009",
			InvalidExpressionAfterDot => "E0010",
			InvalidDefBody => "E0011",
			MissingReturnType => "E0012",
			OverflowingIntegerLiteral => "E0013",
			InvalidDurationUnit => "E0014",
			DurationUnitsOutOfOrder => "E0015",
			OverflowingDurationLiteral => "E0016",
			UnclosedInterpolation => "E0017",
			UnclosedString => "E0018",
			InvalidBytesEscape => "E0019",
			InvalidHexEscape => "E0020",
			BuiltinExpectsPlainString => "E0021",
			ExpectedExpressionAfterSpread => "E0022",
			ExpectedExpressionAfterDefer => "E0023",
			MisplacedRecordSpread => "E0024",
			UnexpectedEOF { .. } => "E0025",
			UnexpectedToken { .. } => "E0026",
			UnexpectedTopLevelToken { .. } => "E0027",
			MisplacedVisibility { .. } => "E0028",
			ExpectedExpression { .. } => "E0029",
			MisplacedRemote => "E0030",
			LeadingDotOutsideUsing => "E0031",
		}
	}

	// Additive only: these surface in the rich renderer (and LSP) without
	// changing the one-line `Display` message, so the analyze suite is
	// unaffected. Kinds whose message already embeds guidance return `None`.
	fn help(&self) -> Option<String> {
		use ParseErrorKind::*;
		let help = match self.kind {
			InvalidBinaryDigit => "binary literals use only `0` and `1` (e.g. `0b1010`).",
			InvalidDecimalDigit => "decimal literals use digits `0`–`9` (e.g. `47`).",
			InvalidHexDigit => "hex literals use `0`–`9` and `a`–`f` (e.g. `0x2a`).",
			InvalidOctalDigit => "octal literals use digits `0`–`7` (e.g. `0o57`).",
			MissingReturnType => "add the return type after `->`, e.g. `fun int -> int`.",
			OverflowingIntegerLiteral => "int literals must fit in a signed 64-bit integer.",
			OverflowingDurationLiteral => {
				"durations are stored as nanoseconds in a signed 64-bit integer."
			}
			UnclosedString => "add a closing `\"`.",
			UnclosedInterpolation => "close the interpolation with `)`, e.g. `\"n = $(to-string n)\"`.",
			BuiltinExpectsPlainString => {
				"write the tag as a plain literal, e.g. `built-in \"io.print\"`."
			}
			InvalidExpressionAfterDot => "use `.field` for a record field or `.0` for a tuple element.",
			LeadingDotOutsideUsing => {
				"wrap it in `using <namespace> { ... }`, or write the access in full (e.g. `css.color`)."
			}

			// The generic `expected }` / `expected {` recoveries are where the
			// common record-vs-block mix-ups surface. The offending token tells
			// us which mistake it almost certainly was, so we can hand back a
			// targeted hint without inventing dedicated error kinds.
			UnexpectedToken {
				expected: Token::RightBrace(..),
				actual: Token::Equal(..),
			} => "record fields are separated with `:`, not `=` (e.g. `{ name: value }`).",
			UnexpectedToken {
				expected: Token::RightBrace(..),
				actual: Token::KeywordLet(..) | Token::KeywordDef(..),
			} => {
				"`{ ... }` is a record literal; for a block of statements, write the body as `fun { ... }`."
			}
			UnexpectedToken {
				expected: Token::RightBrace(..),
				actual: Token::Identifier(..) | Token::Colon(..),
			} => "record fields are separated with commas (e.g. `{ a: 1, b: 2 }`).",
			UnexpectedToken {
				expected: Token::LeftBrace(..),
				actual: Token::Equal(..),
			} => "function literals use a brace body and no arrow: `fun x { x + 1 }`.",

			_ => return None,
		};
		Some(help.to_string())
	}
}
