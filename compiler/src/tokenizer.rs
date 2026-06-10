use crate::errors::{ParseError, ParseErrorKind::*};
use crate::location::Range;
use crate::tokens::{Token, Token::*};
use std::collections::HashMap;

macro_rules! read_string {
	($self:ident, $start:expr, $end:expr) => {
		String::from_utf8($self.source[$start..$end].to_vec()).expect("not utf-8")
	};
}

pub struct Tokenizer<'a> {
	// Comment text keyed by line. Comments are also emitted as `Token::Comment`
	// from `next()`; this parallel index keeps line→text lookup cheap for the
	// formatter and the module's Debug rendering (the tokenizer is the only
	// place with accurate line info, since downstream lookahead skews it).
	pub comments: HashMap<usize, String>,
	source: &'a Vec<u8>,
	length: usize,
	index: usize,
	line: usize,
	line_start_offset: usize,
	expect_import_path: bool,
	string_stack: Vec<usize>,
	interpolation_stack: Vec<usize>,
	// Per active interpolation, the depth of unmatched `(` inside the
	// interpolation's expression. Lets the tokenizer tell apart `)` that
	// closes a nested grouping like `(1 + 2)` from `)` that ends the
	// interpolation itself.
	interpolation_paren_depth: Vec<usize>,
	brace_depth: i32,
	// Lexer-level diagnostics (unclosed string/interpolation, bad escapes,
	// malformed digits). Drained by the parser into the module's error list
	// once tokenization completes — see `Parser::parse_module`.
	pub errors: Vec<ParseError>,
	next_token: Option<Token>,
	indent_level: usize,
}

impl<'a> Tokenizer<'a> {
	pub fn from_source(source: &'a Vec<u8>) -> Self {
		let length = source.len();

		return Tokenizer {
			source,
			length,
			index: 0,
			line: 0,
			line_start_offset: 0,
			expect_import_path: false,
			string_stack: Vec::new(),
			interpolation_stack: Vec::new(),
			interpolation_paren_depth: Vec::new(),
			brace_depth: 0,
			comments: HashMap::new(),
			errors: Vec::new(),
			next_token: None,
			indent_level: 0,
		};
	}

	fn span_to_single_line_range(&self, start: usize, end: usize) -> Range {
		Range::within_line(
			self.line,
			start - self.line_start_offset,
			end - self.line_start_offset,
		)
	}

	// The byte at the cursor, or `None` at end of input. Used by the two-char
	// operator look-aheads (e.g. `+` → `++`, `?` → `??`) so a source that ends
	// mid-operator reads `None` and falls through to the single-char token
	// instead of indexing out of bounds.
	fn peek_byte(&self) -> Option<u8> {
		self.source.get(self.index).copied()
	}
}

impl<'a> Iterator for Tokenizer<'a> {
	type Item = Token;

	fn next(&mut self) -> Option<Token> {
		if self.index >= self.length {
			return None;
		}

		if let Some(next_token) = self.next_token {
			self.next_token = None;
			return Some(next_token);
		}

		// We iterate through all chars in a single loop, appending tokens as we find them.
		// The trickiest parts here are related to string interpolations, since they can
		// be nested arbitrarily deep (e.g. "hello $("Ms. $(name)")"). These parts are
		// commented below.
		'main_loop: while self.index < self.length {
			let start_index = self.index;
			let byte = self.source[start_index];

			if self.string_stack.is_empty() && byte == b'"' {
				// If the string stack is empty and byte is ", we are at the beginning of
				// a brand new string. Save the start index and advance.
				self.string_stack.push(self.index);
				self.index += 1;
				continue;
			}

			if !self.string_stack.is_empty() {
				// If the string stack is not empty, we're somewhere inside a string (maybe
				// in an interpolation, though). We must check if we need to end the string,
				// start/end an interpolation, or just carry on.

				if byte == b'"' && self.string_stack.len() == self.interpolation_stack.len() {
					// If the two stacks have the same size, we must be inside of an interpolation,
					// so the " indicates the beginning of a nested string literal. Save the index
					// in the string stack and advance.
					self.string_stack.push(self.index);
					self.index += 1;
					continue;
				}

				if byte == b'"' {
					// `\"` is an escaped quote, `\\"` is an escaped backslash
					// followed by a string-terminating quote. Count consecutive
					// backslashes — odd count means the quote is escaped.
					let mut backslashes = 0;
					let mut i = self.index;
					while i > 0 && self.source[i - 1] == b'\\' {
						backslashes += 1;
						i -= 1;
					}
					let is_escaped = backslashes % 2 == 1;

					if !is_escaped {
						// Here, the " must indicate the end of a string literal section. Pop from
						// the string stack, add a new token, then advance.
						let start_index = self.string_stack.pop().unwrap() + 1;
						let end_index = self.index;
						self.index += 1;

						return Some(StringLiteral(start_index, end_index));
					}
				}

				if byte == b'$' && start_index + 1 < self.length && self.source[start_index + 1] == b'(' {
					// We must be at the beginning of an interpolation, so create a token for
					// the string literal portion leading up to the interpolation, one for the
					// interpolation start, and add to the interpolation stack.
					let string_start_index = self.string_stack.last().unwrap() + 1;
					let string_end_index = self.index;

					let interpolation_start_start_index = start_index + 1;
					let interpolation_start_end_index = self.index + 2;

					self.interpolation_stack.push(self.index);
					self.interpolation_paren_depth.push(0);
					self.index += 2;

					self.next_token = Some(InterpolationStart(
						interpolation_start_start_index,
						interpolation_start_end_index,
					));

					return Some(StringLiteral(string_start_index, string_end_index));
				}

				if self.interpolation_stack.len() > 0 && byte == b')' {
					let depth = self.interpolation_paren_depth.last_mut().unwrap();
					if *depth > 0 {
						// `)` closes a nested grouping inside the interpolation's
						// expression; let the general RightParen handler emit it.
						*depth -= 1;
					} else {
						// `)` ends the interpolation itself. Fix the string stack
						// so the next string-literal portion starts here, and pop
						// our paren-depth bookkeeping.
						let start_index = self.index;
						let end_index = self.index + 1;

						self.string_stack.pop();
						self.string_stack.push(self.index);

						self.interpolation_stack.pop();
						self.interpolation_paren_depth.pop();
						self.index += 1;

						return Some(InterpolationEnd(start_index, end_index));
					}
				}

				if self.string_stack.len() > self.interpolation_stack.len() {
					// If the string stack is larger than the interpolation stack, we must be
					// inside of a string literal portion. Just advance past this char so we can
					// include it in the string later.
					self.index += 1;
					continue;
				}

				// At this point, we must be inside an interpolation (not a string literal),
				// so continue to collect tokens as we would outside of a string.
			}

			// Bytes literals: `'...'`. No interpolation, no nesting; just scan
			// to the closing quote, treating `\\` and `\'` as escapes that
			// consume both bytes (so `'\''` is a one-byte bytes literal, not
			// an empty one followed by a stray quote).
			if byte == b'\'' {
				let content_start = self.index + 1;
				let mut i = content_start;
				while i < self.length {
					let b = self.source[i];
					if b == b'\\' && i + 1 < self.length {
						// Skip the escaped byte. Validation of which escapes
						// are legal happens in the parser; the tokenizer just
						// needs to not terminate on `\'` or `\\`.
						i += 2;
						continue;
					}
					if b == b'\'' {
						let end = i;
						self.index = i + 1;
						return Some(BytesLiteral(content_start, end));
					}
					i += 1;
				}
				// Unterminated literal: consume to EOF so we don't loop
				// forever, and emit what we have. The parser will surface
				// the lack of a closing quote.
				self.index = self.length;
				return Some(BytesLiteral(content_start, self.length));
			}

			if self.expect_import_path && is_path_char(byte) {
				let mut path_byte = byte;

				while is_path_char(path_byte) {
					self.index += 1;

					if self.index >= self.length {
						break;
					}

					path_byte = self.source[self.index];
				}

				self.expect_import_path = false;

				return Some(Path(start_index, self.index));
			}

			match byte {
				b' ' | b'\r' | b'\t' => {
					self.index += 1;
				}

				b'\n' => {
					self.index += 1;
					self.line += 1;
					self.line_start_offset = self.index;

					let mut indentation_size = 0;
					while self.index < self.length && is_indentation_char(self.source[self.index]) {
						self.index += 1;
						indentation_size += 1;
					}

					if self.indent_level < indentation_size {
						self.next_token = Some(Indent(self.line_start_offset, self.index))
					} else if self.indent_level > indentation_size {
						self.next_token = Some(Outdent(self.line_start_offset, self.index))
					}

					self.indent_level = indentation_size;

					// loop {
					// 	while self.index < self.length && is_indentation_char(self.source[self.index]) {
					// 		self.index += 1;
					// 		indentation_size += 1;
					// 	}

					// 	if self.index < self.length && self.source[self.index] == b'\n' {
					// 		// special case to skip empty lines (or lines with only indent chars)
					// 		self.index += 1;
					// 		self.line += 1;
					// 		self.line_start_offset = self.index;
					// 		indentation_size = 0;
					// 	} else {
					// 		break;
					// 	}
					// }

					// if self.indent_level == indentation_size {
					// 	// no change in indentation
					// 	return Some(LineBreak(self.line_start_offset, self.index));
					// }

					// let is_increase = self.indent_level < indentation_size;

					// self.indent_level = indentation_size;

					// return Some(if is_increase {
					// 	LineBreakWithIndentIncrease(self.line_start_offset, self.index)
					// } else {
					// 	LineBreakWithIndentDecrease(self.line_start_offset, self.index)
					// });

					return Some(LineBreak(start_index, start_index + 1));
				}

				b'(' => {
					if let Some(depth) = self.interpolation_paren_depth.last_mut() {
						*depth += 1;
					}
					self.index += 1;
					return Some(LeftParen(start_index, self.index));
				}

				b')' => {
					self.index += 1;
					return Some(RightParen(start_index, self.index));
				}

				b'{' => {
					self.index += 1;
					self.brace_depth += 1;
					return Some(LeftBrace(start_index, self.index));
				}

				b'}' => {
					self.index += 1;
					self.brace_depth -= 1;
					return Some(RightBrace(start_index, self.index));
				}

				b'[' => {
					self.index += 1;
					return Some(LeftBracket(start_index, self.index));
				}

				b']' => {
					self.index += 1;
					return Some(RightBracket(start_index, self.index));
				}

				b'`' => {
					self.index += 1;
					return Some(Backtick(start_index, self.index));
				}

				b'/' => {
					self.index += 1;
					return Some(ForwardSlash(start_index, self.index));
				}

				b'%' => {
					self.index += 1;
					return Some(Percent(start_index, self.index));
				}

				b'^' => {
					self.index += 1;
					return Some(Caret(start_index, self.index));
				}

				// `$` outside a string literal. Inside a string the `$(` form is
				// already consumed above as InterpolationStart; only the bare
				// glyph reaches this branch (currently used as the end-anchor
				// in regex literals).
				b'$' => {
					self.index += 1;
					return Some(Dollar(start_index, self.index));
				}

				b'-' => {
					self.index += 1;

					if self.peek_byte() == Some(b'>') {
						self.index += 1;
						return Some(Arrow(start_index, self.index));
					}

					// Whitespace-asymmetry rule: when `-` is preceded by
					// whitespace and immediately followed by a non-whitespace
					// char (e.g. `f -x`), emit UnaryMinus so the parser treats
					// it as a prefix on a new arg rather than infix subtract.
					// `a-b`, `a - b`, and `(-x)` all stay as plain Minus.
					let preceded_by_ws = start_index == 0 || is_ws_byte(self.source[start_index - 1]);
					let followed_by_non_ws = self.index < self.length && !is_ws_byte(self.source[self.index]);

					if preceded_by_ws && followed_by_non_ws {
						return Some(UnaryMinus(start_index, self.index));
					}

					return Some(Minus(start_index, self.index));
				}

				b'+' => {
					self.index += 1;

					if self.peek_byte() == Some(b'+') {
						self.index += 1;
						return Some(DoublePlus(start_index, self.index));
					}

					return Some(Plus(start_index, self.index));
				}

				b',' => {
					self.index += 1;
					return Some(Comma(start_index, self.index));
				}

				b'~' => {
					self.index += 1;
					return Some(Tilde(start_index, self.index));
				}

				b'?' => {
					self.index += 1;

					if self.peek_byte() == Some(b'?') {
						self.index += 1;
						return Some(DoubleQuestion(start_index, self.index));
					}

					return Some(Question(start_index, self.index));
				}

				b'!' => {
					self.index += 1;

					if self.peek_byte() == Some(b'=') {
						self.index += 1;
						return Some(BangEqual(start_index, self.index));
					}

					return Some(Bang(start_index, self.index));
				}

				b'=' => {
					self.index += 1;

					if self.peek_byte() == Some(b'=') {
						self.index += 1;
						return Some(DoubleEqual(start_index, self.index));
					}

					return Some(Equal(start_index, self.index));
				}

				b'*' => {
					self.index += 1;

					if self.peek_byte() == Some(b'*') {
						self.index += 1;
						return Some(DoubleStar(start_index, self.index));
					}

					return Some(Star(start_index, self.index));
				}

				b'.' => {
					self.index += 1;

					if self.peek_byte() == Some(b'.') {
						self.index += 1;
						if self.peek_byte() == Some(b'.') {
							self.index += 1;
							return Some(TripleDot(start_index, self.index));
						}
						return Some(DoubleDot(start_index, self.index));
					}

					return Some(Dot(start_index, self.index));
				}

				b'&' => {
					self.index += 1;

					if self.peek_byte() == Some(b'&') {
						self.index += 1;
						return Some(DoubleAnd(start_index, self.index));
					}

					return Some(And(start_index, self.index));
				}

				b'|' => {
					self.index += 1;

					if self.peek_byte() == Some(b'|') {
						self.index += 1;
						return Some(DoublePipe(start_index, self.index));
					}

					if self.peek_byte() == Some(b'>') {
						self.index += 1;
						return Some(PipeArrow(start_index, self.index));
					}

					return Some(Pipe(start_index, self.index));
				}

				b':' => {
					self.index += 1;

					if self.peek_byte() == Some(b':') {
						self.index += 1;
						return Some(DoubleColon(start_index, self.index));
					}

					return Some(Colon(start_index, self.index));
				}

				b'<' => {
					self.index += 1;

					if self.peek_byte() == Some(b'=') {
						self.index += 1;
						return Some(LeftAngleEqual(start_index, self.index));
					}

					if self.peek_byte() == Some(b'<') {
						self.index += 1;
						return Some(DoubleLeftAngle(start_index, self.index));
					}

					return Some(LeftAngle(start_index, self.index));
				}

				b'>' => {
					self.index += 1;

					if self.peek_byte() == Some(b'=') {
						self.index += 1;
						return Some(RightAngleEqual(start_index, self.index));
					}

					if self.peek_byte() == Some(b'>') {
						self.index += 1;
						return Some(DoubleRightAngle(start_index, self.index));
					}

					return Some(RightAngle(start_index, self.index));
				}

				b'#' => {
					while self.index < self.length && self.source[self.index] != b'\n' {
						self.index += 1;
					}

					let comment = read_string!(self, start_index + 1, self.index);
					self.comments.insert(self.line, comment);

					// Emit the comment as a token (span includes the `#`).
					// Consumers that don't care about comments — i.e. the parser —
					// skip them as trivia, just like line breaks.
					return Some(Comment(start_index, self.index));
				}

				_ if is_identifier_start_char(byte) => {
					while self.index < self.length && is_identifier_char(self.source[self.index]) {
						self.index += 1;
					}

					let value = &self.source[start_index..self.index];

					let constructor = match value {
						b"_" => Underscore,

						b"true" => BoolTrue,
						b"false" => BoolFalse,

						b"alias" => KeywordAlias,
						b"as" => KeywordAs,
						b"built-in" => KeywordBuiltin,
						b"def" => KeywordDef,
						b"defer" => KeywordDefer,
						b"else" => KeywordElse,
						b"enum" => KeywordEnum,
						b"fun" => KeywordFun,
						b"if" => KeywordIf,
						b"implement" => KeywordImplement,
						b"in" => KeywordIn,
						b"is" => KeywordIs,
						b"let" => KeywordLet,
						b"manual" => KeywordManual,
						b"opaque" => KeywordOpaque,
						b"public" => KeywordPublic,
						b"remote" => KeywordRemote,
						b"scope" => KeywordScope,
						b"try" => KeywordTry,
						b"trait" => KeywordTrait,
						b"use" => KeywordUse,
						b"using" => KeywordUsing,
						b"when" => KeywordWhen,
						b"where" => KeywordWhere,
						b"while" => KeywordWhile,

						// Anything else is just an identifier:
						_ => Identifier,
					};

					// if constructor == KeywordUse {
					// 	self.expect_import_path = true;
					// }

					return Some(constructor(start_index, self.index));
				}

				_ if is_digit(byte) => {
					if byte == b'0' {
						match self.source.get(self.index + 1) {
							Some(b'b') | Some(b'B') => {
								self.index += 2;

								while self.index < self.length && is_identifier_char(self.source[self.index]) {
									if self.source[self.index] != b'0' && self.source[self.index] != b'1' {
										let error_start = self.index;

										while self.index < self.length && !self.source[self.index].is_ascii_whitespace()
										{
											self.index += 1;
										}

										self.errors.push(ParseError {
											range: self.span_to_single_line_range(error_start, self.index),
											kind: InvalidBinaryDigit,
										});

										continue 'main_loop;
									}

									self.index += 1;
								}

								return Some(BinaryDigits(start_index, self.index));
							}

							Some(b'x') | Some(b'X') => {
								self.index += 2;

								while self.index < self.length && is_identifier_char(self.source[self.index]) {
									if !self.source[self.index].is_ascii_hexdigit() {
										let error_start = self.index;

										while self.index < self.length && !self.source[self.index].is_ascii_whitespace()
										{
											self.index += 1;
										}

										self.errors.push(ParseError {
											range: self.span_to_single_line_range(error_start, self.index),
											kind: InvalidHexDigit,
										});

										continue 'main_loop;
									}

									self.index += 1;
								}

								return Some(HexDigits(start_index, self.index));
							}

							Some(b'o') | Some(b'O') => {
								self.index += 2;

								while self.index < self.length && is_identifier_char(self.source[self.index]) {
									if self.source[self.index] < 48 || self.source[self.index] > 55 {
										let error_start = self.index;

										while self.index < self.length && !self.source[self.index].is_ascii_whitespace()
										{
											self.index += 1;
										}

										self.errors.push(ParseError {
											range: self.span_to_single_line_range(error_start, self.index),
											kind: InvalidOctalDigit,
										});

										continue 'main_loop;
									}

									self.index += 1;
								}

								return Some(OctalDigits(start_index, self.index));
							}

							_ => {}
						}
					}

					while self.index < self.length && self.source[self.index].is_ascii_digit() {
						self.index += 1;
					}

					// A unit letter immediately after the digits (no space) makes
					// this a duration literal: `5s`, `2m20s`, `3h2m10s`. Consume the
					// whole run of digits and ASCII letters; the parser splits it
					// into `<amount><unit>` segments and validates order/range.
					if self.index < self.length && is_duration_unit_start(self.source[self.index]) {
						while self.index < self.length
							&& (self.source[self.index].is_ascii_digit()
								|| self.source[self.index].is_ascii_alphabetic())
						{
							self.index += 1;
						}

						return Some(DurationLiteral(start_index, self.index));
					}

					// Any other trailing identifier char means the digits were
					// malformed (e.g. `12abc`).
					if self.index < self.length && is_identifier_char(self.source[self.index]) {
						let error_start = self.index;

						while self.index < self.length && !self.source[self.index].is_ascii_whitespace() {
							self.index += 1;
						}

						self.errors.push(ParseError {
							range: self.span_to_single_line_range(error_start, self.index),
							kind: InvalidDecimalDigit,
						});

						continue 'main_loop;
					}

					return Some(DecimalDigits(start_index, self.index));
				}

				other_char => {
					self.index += 1;
					return Some(Unexpected(other_char, start_index, self.index));
				}
			};
		}

		if !self.interpolation_stack.is_empty() {
			let start_index = self.interpolation_stack.pop().unwrap();

			self.errors.push(ParseError {
				range: self.span_to_single_line_range(start_index, self.index),
				kind: UnclosedInterpolation,
			});
		}

		if !self.string_stack.is_empty() {
			let start_index = self.string_stack.pop().unwrap();

			self.errors.push(ParseError {
				range: self.span_to_single_line_range(start_index, start_index + 1),
				kind: UnclosedString,
			});
		}

		None
	}
}

fn is_identifier_start_char(byte: u8) -> bool {
	match byte {
		_ if is_digit(byte) => false,
		_ => is_identifier_char(byte),
	}
}

fn is_identifier_char(byte: u8) -> bool {
	// we want to allow for as many valid identifiers as we can (i.e. not just ASCII!),
	// so we only exclude chars here that are whitespace/punctuation/operators/etc.
	match byte {
		_ if byte.is_ascii_whitespace() => false,
		_ if byte.is_ascii_control() => false,
		b':' | b'|' | b'.' | b'*' | b'\\' | b'/' | b'+' | b'=' | b'<' | b'>' | b'~' | b'!' | b'%'
		| b'&' | b'@' | b'^' | b'?' | b'"' | b'#' | b'$' | b'\'' | b'(' | b')' | b',' | b';' | b'`'
		| b'[' | b']' | b'{' | b'}' => false,
		_ => true,
	}
}

// First letter of a time unit (`ns`, `us`, `ms`, `s`, `m`, `h`, `d`). Used to
// decide whether digits begin a duration literal rather than a plain number.
fn is_duration_unit_start(byte: u8) -> bool {
	matches!(byte, b'n' | b'u' | b'm' | b's' | b'h' | b'd')
}

fn is_digit(byte: u8) -> bool {
	match byte {
		b'0'..=b'9' => true,
		_ => false,
	}
}

fn is_indentation_char(byte: u8) -> bool {
	match byte {
		b'\t' => true,
		b' ' => true,
		_ => false,
	}
}

fn is_ws_byte(byte: u8) -> bool {
	matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_path_char(byte: u8) -> bool {
	match byte {
		b'@' | b'\\' | b'?' | b'%' | b'*' | b':' | b'"' | b'<' | b'>' => false,
		b if b.is_ascii_whitespace() => false,
		_ => true,
	}
}
