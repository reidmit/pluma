use crate::ast::*;
use crate::errors::*;
use crate::location::Point;
use crate::location::Range;
use crate::tokenizer::Tokenizer;
use crate::tokens::Token;
use crate::types::*;
use std::collections::{HashMap, VecDeque};

fn hex_digit(byte: u8) -> Option<u8> {
	match byte {
		b'0'..=b'9' => Some(byte - b'0'),
		b'a'..=b'f' => Some(byte - b'a' + 10),
		b'A'..=b'F' => Some(byte - b'A' + 10),
		_ => None,
	}
}

macro_rules! current_token_is {
	($self:ident, $tokType:path) => {
		match $self.current_token {
			Some($tokType(..)) => true,
			_ => false,
		}
	};
}

macro_rules! expect_token_and_advance {
	($self:ident, $tokType:path) => {
		match $self.current_token {
			Some($tokType(start, end)) => {
				$self.advance();
				let start_point = $self.offset_to_point(start);
				let end_point = $self.offset_to_point(end);
				(start_point, end_point)
			}
			Some(tok) => {
				let (start, end) = tok.get_span();
				let start_point = $self.offset_to_point(start);
				let end_point = $self.offset_to_point(end);
				return $self.error(ParseError {
					range: Range::between(start_point, end_point),
					kind: ParseErrorKind::UnexpectedToken {
						actual: tok,
						expected: $tokType(0, 0),
					},
				});
			}
			None => {
				return $self.error(ParseError {
					range: Range::collapsed($self.current_line, 0),
					kind: ParseErrorKind::UnexpectedEOF {
						expected: $tokType(0, 0),
					},
				});
			}
		}
	};
}

macro_rules! read_string {
	($self:ident, $start_offset:expr, $end_offset:expr) => {
		String::from_utf8($self.source[$start_offset..$end_offset].to_vec()).expect("not utf-8")
	};
}

macro_rules! read_string_with_escapes {
	($self:ident, $start:expr, $end:expr) => {
		read_string!($self, $start, $end)
			.replace("\\\"", "\"")
			.replace("\\\\", "\\")
			.replace("\\t", "\t")
			.replace("\\r", "\r")
			.replace("\\n", "\n")
	};
}

pub struct Parser<'a> {
	source: &'a Vec<u8>,
	tokenizer: Tokenizer<'a>,
	errors: Vec<ParseError>,
	current_token: Option<Token>,
	prev_token: Option<Token>,
	// Tokens pulled from the tokenizer ahead of where the parser is sitting,
	// populated by `peek_past_breaks` and drained by `advance`. Lets us look
	// past line-break tokens to decide whether an infix operator continues
	// the current expression, without committing to consuming the breaks.
	lookahead: VecDeque<Token>,
	current_line: usize,
	line_start_offsets: HashMap<usize, usize>,
}

impl<'a> Parser<'a> {
	pub fn new(source: &'a Vec<u8>, tokenizer: Tokenizer<'a>) -> Parser<'a> {
		return Parser {
			source,
			tokenizer,
			errors: Vec::new(),
			current_token: None,
			prev_token: None,
			lookahead: VecDeque::new(),
			current_line: 0,
			line_start_offsets: HashMap::from_iter(vec![(0, 0)]),
		};
	}

	pub fn parse_module(&mut self) -> (ModuleNode, HashMap<usize, String>, Vec<ParseError>) {
		let mut uses = Vec::new();
		let mut body = Vec::new();

		// Read the first token
		self.advance();

		// `use` declarations must come first; once we see a `def` we stop
		// looking for them. On error inside a `use`, sync past the
		// malformed declaration so the next one (or the first def) still
		// gets parsed cleanly without a cascade.
		loop {
			self.skip_line_breaks();
			match self.current_token {
				Some(Token::KeywordUse(..)) => match self.parse_use() {
					Some(u) => uses.push(u),
					None => self.synchronize_to_top_level(),
				},
				_ => break,
			}
		}

		loop {
			self.skip_line_breaks();

			let Some(tok) = self.current_token else { break };

			// Stray non-keyword at the top level — report and sync past it
			// so the next definition still gets parsed.
			if !Self::is_top_level_start(tok) {
				let (s, e) = tok.get_span();
				let _: Option<()> = self.error(ParseError {
					range: Range::between(self.offset_to_point(s), self.offset_to_point(e)),
					kind: ParseErrorKind::UnexpectedTopLevelToken { actual: tok },
				});
				self.synchronize_to_top_level();
				continue;
			}

			match self.parse_definition() {
				Some(definition) => body.push(definition),
				// A top-level definition errored partway through. The
				// inner parser already reported the diagnostic; sync past
				// the stale state so subsequent definitions still parse.
				None => self.synchronize_to_top_level(),
			}
		}

		let start = uses
			.first()
			.map(|u| u.range.start)
			.or_else(|| body.first().map(|d| d.range.start))
			.unwrap_or_else(Point::zero);
		let end = body
			.last()
			.map(|d| d.range.end)
			.or_else(|| uses.last().map(|u| u.range.end))
			.unwrap_or_else(Point::zero);

		(
			ModuleNode {
				range: Range::between(start, end),
				uses,
				body,
			},
			self.tokenizer.comments.clone(),
			self.errors.clone(),
		)
	}

	fn parse_use(&mut self) -> Option<UseNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordUse);

		let mut path = Vec::new();
		path.push(self.expect_identifier()?);

		while matches!(self.current_token, Some(Token::Dot(..))) {
			self.advance();
			path.push(self.expect_identifier()?);
		}

		let alias = if matches!(self.current_token, Some(Token::KeywordAs(..))) {
			self.advance();
			Some(self.expect_identifier()?)
		} else {
			None
		};

		let end = alias
			.as_ref()
			.map(|a| a.range.end)
			.unwrap_or_else(|| path.last().unwrap().range.end);

		Some(UseNode {
			range: Range::between(start, end),
			path,
			alias,
		})
	}

	fn advance(&mut self) {
		self.prev_token = self.current_token;
		self.current_token = self
			.lookahead
			.pop_front()
			.or_else(|| self.next_significant_token());
	}

	// Pull the next token from the tokenizer, skipping comments. The parser
	// treats comments as trivia — like line breaks — and never sees them in the
	// token stream. This is the single point where comments are filtered, so
	// the grammar productions don't have to. (Their text/spans are still
	// recorded by the tokenizer: `comments` for the formatter, `Token::Comment`
	// for the LSP's highlighting pass.)
	fn next_significant_token(&mut self) -> Option<Token> {
		loop {
			match self.tokenizer.next() {
				Some(Token::Comment(..)) => continue,
				other => return other,
			}
		}
	}

	// Look past any line-break-ish tokens (LineBreak/Indent/Outdent) starting
	// from `current_token` and return the first non-break token, without
	// consuming anything. Pulled tokens are buffered so a subsequent
	// `advance` / `skip_line_breaks` still sees them.
	fn peek_past_breaks(&mut self) -> Option<Token> {
		let is_break = |t: &Token| {
			matches!(
				t,
				Token::LineBreak(..) | Token::Indent(..) | Token::Outdent(..)
			)
		};

		// `current_token` itself counts as the first slot to inspect; if it's
		// already non-break we don't need to look further.
		if let Some(t) = self.current_token {
			if !is_break(&t) {
				return Some(t);
			}
		}

		for t in self.lookahead.iter() {
			if !is_break(t) {
				return Some(*t);
			}
		}

		loop {
			match self.next_significant_token() {
				Some(t) => {
					self.lookahead.push_back(t);
					if !is_break(&t) {
						return Some(t);
					}
				}
				None => return None,
			}
		}
	}

	fn skip_line_breaks(&mut self) {
		loop {
			match &self.current_token {
				Some(Token::LineBreak(.., end_offset)) => {
					self.current_line += 1;
					self
						.line_start_offsets
						.insert(self.current_line, *end_offset);
					self.advance();
				}
				Some(Token::Indent(..)) => self.advance(),
				Some(Token::Outdent(..)) => self.advance(),
				_ => break,
			}
		}
	}

	fn current_token_span(&self) -> (usize, usize) {
		match self.current_token {
			Some(token) => token.get_span(),
			_ => match self.prev_token {
				Some(token) => token.get_span(),
				_ => (0, 0),
			},
		}
	}

	fn current_token_points(&self) -> (Point, Point) {
		let (start_offset, end_offset) = self.current_token_span();
		(
			self.offset_to_point(start_offset),
			self.offset_to_point(end_offset),
		)
	}

	fn offset_to_point(&self, offset: usize) -> Point {
		Point::at(
			self.current_line,
			offset - self.line_start_offsets.get(&self.current_line).unwrap(),
		)
	}

	fn point_to_offset(&self, point: Point) -> usize {
		let line_start_offset = self.line_start_offsets.get(&point.line).unwrap();
		line_start_offset + point.col
	}

	fn span_to_single_line_range(&self, start: usize, end: usize) -> Range {
		let current_line_start = self.line_start_offsets.get(&self.current_line).unwrap();
		Range::within_line(
			self.current_line,
			start - current_line_start,
			end - current_line_start,
		)
	}

	fn error<A>(&mut self, err: ParseError) -> Option<A> {
		self.errors.push(err);
		None
	}

	fn is_top_level_start(tok: Token) -> bool {
		matches!(
			tok,
			Token::KeywordDef(..)
				| Token::KeywordEnum(..)
				| Token::KeywordAlias(..)
				| Token::KeywordTrait(..)
				| Token::KeywordImplement(..)
				| Token::KeywordPublic(..)
				| Token::KeywordOpaque(..)
		)
	}

	// Panic-mode recovery at the top-level boundary. Skip tokens until we
	// land on the start of the next definition, tracking brace depth so we
	// don't stop at a `def` inside a partially-consumed trait/implement
	// body. The first token is always skipped past so the caller's failing
	// position can't pin us in place.
	fn synchronize_to_top_level(&mut self) {
		let mut brace_depth: i32 = 0;
		let mut just_started = true;

		loop {
			match self.current_token {
				None => return,
				Some(tok) if !just_started && brace_depth <= 0 && Self::is_top_level_start(tok) => {
					return;
				}
				Some(Token::LeftBrace(..)) => {
					brace_depth += 1;
					self.advance();
				}
				Some(Token::RightBrace(..)) => {
					brace_depth -= 1;
					self.advance();
				}
				Some(Token::LineBreak(..)) | Some(Token::Indent(..)) | Some(Token::Outdent(..)) => {
					self.skip_line_breaks();
				}
				_ => self.advance(),
			}
			just_started = false;
		}
	}

	fn parse_body_expressions(&mut self) -> Option<Vec<ExprNode>> {
		let mut body = Vec::new();

		loop {
			self.skip_line_breaks();

			// `try Pattern = Expr` is a body-only form. It absorbs every
			// remaining expression of the surrounding block into its `rest`
			// field — at analyze time the rest becomes the continuation
			// closure passed to `<carrier>.then`. Once parsed, no more
			// siblings can follow at this level.
			if current_token_is!(self, Token::KeywordTry) {
				let try_expr = self.parse_try_with_rest()?;
				body.push(try_expr);
				break;
			}

			if let Some(node) = self.parse_expression() {
				body.push(node);
			} else {
				break;
			}
		}

		Some(body)
	}

	// Parse `try Pattern = Expr` and collect everything after it (through
	// the end of the current block) into `rest`. Nested `try`s in `rest`
	// are handled by recursing through `parse_body_expressions`.
	fn parse_try_with_rest(&mut self) -> Option<ExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordTry);

		let pattern = match self.parse_pattern() {
			Some(p) => p,
			None => {
				self.expect_identifier()?;
				return None;
			}
		};

		expect_token_and_advance!(self, Token::Equal);

		let value = self.parse_expression()?;

		let rest = self.parse_body_expressions()?;

		let end = rest.last().map(|e| e.range.end).unwrap_or(value.range.end);

		let range = Range::between(start, end);

		Some(ExprNode {
			range,
			kind: ExprKind::Try(TryNode {
				range,
				pattern,
				value: Box::new(value),
				rest,
				pattern_ty: Type::Unknown,
				task_carrier: false,
			}),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_fun(&mut self) -> Option<FunNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordFun);

		let mut params = Vec::new();

		// TODO: allow patterns here, not just identifiers
		while current_token_is!(self, Token::Identifier) {
			let ident = self.parse_identifier()?;

			params.push(FunParamNode {
				ident,
				ty: Type::Unknown,
			});
		}

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		self.skip_line_breaks();

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(FunNode {
			range: Range::between(start, end),
			params,
			body,
		})
	}

	fn parse_decimal_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::DecimalDigits);

		if current_token_is!(self, Token::Dot) {
			self.advance();

			let (_, end) = expect_token_and_advance!(self, Token::DecimalDigits);
			let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

			let str_value = read_string!(self, start_offset, end_offset);
			let float_value = str_value.parse::<f64>().unwrap();

			return Some(LiteralNode {
				kind: LiteralKind::FloatDecimal(float_value),
				range: Range::between(start, end),
			});
		}

		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let value = self.parse_numeric_literal(start_offset, end_offset, 10);

		Some(LiteralNode {
			kind: LiteralKind::IntDecimal(value),
			range: Range::between(start, end),
		})
	}

	/// Parse a `DurationLiteral` token (e.g. `5s`, `2m20s`, `3h2m10s`) into a
	/// `LiteralKind::Duration` carrying the total nanoseconds. The token text is
	/// a run of `<amount><unit>` segments; units must each appear at most once
	/// and in strictly descending order of magnitude (d > h > m > s > ms > us >
	/// ns). On a malformed literal we report the first problem and yield a
	/// zero-duration node so analysis can continue.
	fn parse_duration_literal(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::DurationLiteral);
		let range = Range::between(start, end);
		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));
		let text = read_string!(self, start_offset, end_offset);
		let bytes = text.as_bytes();

		let mut total: i64 = 0;
		let mut prev_rank: Option<u8> = None;
		let mut reported = false;
		let mut i = 0;

		while i < bytes.len() {
			let mut amount: i64 = 0;
			while i < bytes.len() && bytes[i].is_ascii_digit() {
				match amount
					.checked_mul(10)
					.and_then(|a| a.checked_add((bytes[i] - b'0') as i64))
				{
					Some(next) => amount = next,
					None => self.report_once(
						&mut reported,
						ParseErrorKind::OverflowingDurationLiteral,
						range,
					),
				}
				i += 1;
			}

			let unit_start = i;
			while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
				i += 1;
			}

			if unit_start == i {
				// Digits with no following unit (e.g. a trailing `5s5`).
				self.report_once(&mut reported, ParseErrorKind::InvalidDurationUnit, range);
				break;
			}

			let (per_unit, rank): (i64, u8) = match &text[unit_start..i] {
				"d" => (86_400_000_000_000, 6),
				"h" => (3_600_000_000_000, 5),
				"m" => (60_000_000_000, 4),
				"s" => (1_000_000_000, 3),
				"ms" => (1_000_000, 2),
				"us" => (1_000, 1),
				"ns" => (1, 0),
				_ => {
					self.report_once(&mut reported, ParseErrorKind::InvalidDurationUnit, range);
					break;
				}
			};

			if prev_rank.is_some_and(|pr| rank >= pr) {
				self.report_once(
					&mut reported,
					ParseErrorKind::DurationUnitsOutOfOrder,
					range,
				);
			}
			prev_rank = Some(rank);

			match amount
				.checked_mul(per_unit)
				.and_then(|seg| total.checked_add(seg))
			{
				Some(next) => total = next,
				None => self.report_once(
					&mut reported,
					ParseErrorKind::OverflowingDurationLiteral,
					range,
				),
			}
		}

		Some(LiteralNode {
			kind: LiteralKind::Duration(if reported { 0 } else { total }),
			range,
		})
	}

	/// Push a parse error unless one has already been reported for the current
	/// construct (tracked by the caller's `reported` flag). Keeps a single
	/// malformed literal from producing a cascade of diagnostics.
	fn report_once(&mut self, reported: &mut bool, kind: ParseErrorKind, range: Range) {
		if !*reported {
			self.errors.push(ParseError { range, kind });
			*reported = true;
		}
	}

	fn parse_binary_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::BinaryDigits);
		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let value = self.parse_numeric_literal(start_offset + 2, end_offset, 2);

		Some(LiteralNode {
			kind: LiteralKind::IntBinary(value),
			range: Range::between(start, end),
		})
	}

	fn parse_octal_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::OctalDigits);
		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let value = self.parse_numeric_literal(start_offset + 2, end_offset, 8);

		Some(LiteralNode {
			kind: LiteralKind::IntOctal(value),
			range: Range::between(start, end),
		})
	}

	fn parse_hex_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::HexDigits);
		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let value = self.parse_numeric_literal(start_offset + 2, end_offset, 16);

		Some(LiteralNode {
			kind: LiteralKind::IntHex(value),
			range: Range::between(start, end),
		})
	}

	fn parse_bool(&mut self) -> Option<ExprNode> {
		let (start, end, value) = match &self.current_token {
			Some(Token::BoolTrue(start, end)) => (*start, *end, true),
			Some(Token::BoolFalse(start, end)) => (*start, *end, false),
			_ => unreachable!(),
		};

		self.advance();

		Some(ExprNode {
			range: self.span_to_single_line_range(start, end),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
			kind: ExprKind::Literal(LiteralNode {
				range: self.span_to_single_line_range(start, end),
				kind: LiteralKind::Bool(value),
			}),
		})
	}

	fn parse_expression(&mut self) -> Option<ExprNode> {
		self.parse_expression_with_binding_power(0)
	}

	fn parse_expression_with_binding_power(&mut self, min_bp: u8) -> Option<ExprNode> {
		let mut lhs_expr = match self.current_token {
			Some(Token::LeftParen(..)) => self.parse_parenthetical(),
			Some(Token::LeftBrace(..)) => self.parse_record(),
			Some(Token::LeftBracket(..)) => self.parse_list(),
			Some(Token::Backtick(..)) => self.parse_regular_expression(),
			Some(Token::StringLiteral(..)) => self.parse_string(),
			Some(Token::BytesLiteral(..)) => self.parse_bytes(),
			Some(Token::BoolTrue(..)) => self.parse_bool(),
			Some(Token::BoolFalse(..)) => self.parse_bool(),
			Some(Token::KeywordWhen(..)) => self.parse_when_expression().map(|when_node| ExprNode {
				range: when_node.range,
				kind: ExprKind::When(when_node),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::KeywordIf(..)) => self.parse_if_expression().map(|if_node| ExprNode {
				range: if_node.range,
				kind: ExprKind::If(if_node),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::KeywordWhile(..)) => self.parse_while_expression().map(|while_node| ExprNode {
				range: while_node.range,
				kind: ExprKind::While(while_node),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::KeywordLet(..)) => self.parse_let_expression().map(|let_node| ExprNode {
				range: let_node.range,
				kind: ExprKind::Let(let_node),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::KeywordScope(..)) | Some(Token::KeywordManual(..)) => {
				self.parse_scope_expression().map(|scope_node| ExprNode {
					range: scope_node.range,
					kind: ExprKind::Scope(scope_node),
					ty: Type::Unknown,
					trait_dispatch: None,
					dispatch_sink: None,
				})
			}
			Some(Token::KeywordDefer(..)) => self.parse_defer(),
			Some(Token::KeywordFun(..)) => self.parse_fun().map(|fun_node| ExprNode {
				range: fun_node.range,
				kind: ExprKind::Fun(fun_node),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::KeywordBuiltin(..)) => self.parse_builtin(),
			Some(Token::Identifier(..)) => self.parse_identifier().map(|ident| ExprNode {
				range: ident.range,
				kind: ExprKind::Identifier(ident),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::DurationLiteral(..)) => self.parse_duration_literal().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(Token::HexDigits(..)) => self.parse_hex_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			}),
			Some(
				t @ Token::Minus(start, ..) | t @ Token::UnaryMinus(start, ..) | t @ Token::Bang(start, ..),
			) => {
				// these are prefix unary operators!
				let operator = match t {
					Token::UnaryMinus(..) => Operator::SubtractionOrNegation,
					_ => Operator::from_token(t).unwrap(),
				};
				self.advance();

				let start_point = self.offset_to_point(start);

				// make sure to parse the expression following the operator with
				// the correct binding power:
				let (_, right_bp) = operator.prefix_binding_power();
				let rhs_expr = self.parse_expression_with_binding_power(right_bp)?;

				Some(ExprNode {
					range: Range::between(start_point, rhs_expr.range.end),
					kind: ExprKind::UnaryOperation {
						op: operator,
						right: Box::new(rhs_expr),
					},
					ty: Type::Unknown,
					trait_dispatch: None,
					dispatch_sink: None,
				})
			}
			_ => None,
		}?;

		loop {
			// Line breaks normally terminate an expression, but if the next
			// non-break token is an infix operator we let it continue across
			// the break (so `x\n  | f` parses like `x | f`). We don't extend
			// this to FunctionCall — `foo\nbar` must stay as two statements,
			// not `foo bar`.
			if matches!(
				self.current_token,
				Some(Token::LineBreak(..) | Token::Indent(..) | Token::Outdent(..))
			) {
				match self.peek_past_breaks().and_then(Operator::from_token) {
					Some(_) => self.skip_line_breaks(),
					None => break,
				}
			}

			let operator = match self.current_token {
				Some(token) => match Operator::from_token(token) {
					Some(op) => {
						self.skip_line_breaks();
						op
					}
					_ if token.can_start_expression() => Operator::FunctionCall,
					_ => break,
				},
				_ => break,
			};

			if let Some((left_bp, right_bp)) = operator.infix_binding_power() {
				if left_bp < min_bp {
					break;
				}

				if let Operator::FunctionCall = operator {
					// special case: function calls don't have a real operator token,
					// and they may take any number of args, so we handle all that here
					let mut args = Vec::new();

					loop {
						// A plain `Minus` following a complete operand is binary
						// subtraction, not a negated argument: stop collecting args
						// so `f a - b` reads as `(f a) - b` (the infix loop picks the
						// `-` up). `UnaryMinus` — the whitespace-asymmetric `f -x`
						// form the tokenizer emits — still begins a negated argument.
						if matches!(self.current_token, Some(Token::Minus(..))) {
							break;
						}
						match self.parse_expression_with_binding_power(right_bp) {
							Some(arg_expr) => args.push(arg_expr),
							None => break,
						}
					}

					// We entered FunctionCall because `can_start_expression`
					// said the next token could begin one, but couldn't actually
					// parse an arg — give up and let the outer parser report a
					// useful error on whatever's there.
					if args.is_empty() {
						break;
					}

					let range = Range::between(lhs_expr.range.start, args.last().unwrap().range.end);

					lhs_expr = ExprNode {
						range,
						kind: ExprKind::Call(CallNode {
							range,
							callee: Box::new(lhs_expr),
							args,
							dict_args: Vec::new(),
						}),
						ty: Type::Unknown,
						trait_dispatch: None,
						dispatch_sink: None,
					};
				} else {
					let op_pos = self.current_token_points();

					// advance past the operator token
					self.advance();

					let rhs_expr = self.parse_expression_with_binding_power(right_bp)?;

					if let Operator::IndexAccess = operator {
						// special case: the [ operator needs a closing ]
						expect_token_and_advance!(self, Token::RightBracket);
					}

					if let Operator::FieldAccess = operator {
						// another special case: element/field access nodes look a little
						// different to make analysis easier
						lhs_expr = self.make_element_or_field_access(lhs_expr, rhs_expr)?;
						continue;
					}

					lhs_expr = ExprNode {
						range: Range::between(lhs_expr.range.start, rhs_expr.range.end),
						kind: ExprKind::BinaryOperation {
							op: OperatorNode {
								range: Range::between(op_pos.0, op_pos.1),
								kind: operator,
							},
							left: Box::new(lhs_expr),
							right: Box::new(rhs_expr),
						},
						ty: Type::Unknown,
						trait_dispatch: None,
						dispatch_sink: None,
					};
				}

				continue;
			}

			break;
		}

		Some(lhs_expr)
	}

	fn make_element_or_field_access(
		&mut self,
		lhs_expr: ExprNode,
		rhs_expr: ExprNode,
	) -> Option<ExprNode> {
		match rhs_expr.kind {
			ExprKind::Literal(LiteralNode {
				kind: LiteralKind::IntDecimal(index),
				..
			}) => Some(ExprNode {
				range: Range::between(lhs_expr.range.start, rhs_expr.range.end),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
				kind: ExprKind::ElementAccess {
					receiver: lhs_expr.into(),
					index,
				},
			}),

			ExprKind::Identifier(ident) => Some(ExprNode {
				range: Range::between(lhs_expr.range.start, rhs_expr.range.end),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
				kind: ExprKind::FieldAccess {
					receiver: lhs_expr.into(),
					field: ident,
				},
			}),

			_ => {
				self.error::<ExprNode>(ParseError {
					range: rhs_expr.range,
					kind: ParseErrorKind::InvalidExpressionAfterDot,
				});
				None
			}
		}
	}

	fn parse_identifier(&mut self) -> Option<IdentifierNode> {
		let (start, end) = match self.current_token {
			Some(Token::Identifier(start, end)) => {
				self.advance();
				(start, end)
			}
			_ => return None,
		};

		let name = read_string!(self, start, end);

		Some(IdentifierNode {
			range: self.span_to_single_line_range(start, end),
			name,
		})
	}

	// Like `parse_identifier`, but reports a diagnostic and returns `None`
	// if the current token isn't an identifier. Use at sites where an
	// identifier is required (`def NAME`, `let NAME`, function params,
	// etc.). Use `parse_identifier` only when the absence of an identifier
	// is a legitimate signal to try something else.
	fn expect_identifier(&mut self) -> Option<IdentifierNode> {
		match self.parse_identifier() {
			Some(node) => Some(node),
			None => match self.current_token {
				Some(tok) => {
					let (s, e) = tok.get_span();
					self.error(ParseError {
						range: Range::between(self.offset_to_point(s), self.offset_to_point(e)),
						kind: ParseErrorKind::UnexpectedToken {
							actual: tok,
							expected: Token::Identifier(0, 0),
						},
					})
				}
				None => self.error(ParseError {
					range: Range::collapsed(self.current_line, 0),
					kind: ParseErrorKind::UnexpectedEOF {
						expected: Token::Identifier(0, 0),
					},
				}),
			},
		}
	}

	fn parse_if_expression(&mut self) -> Option<IfNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordIf);

		let condition = self.parse_expression()?;

		expect_token_and_advance!(self, Token::KeywordIs);

		let pattern = self.parse_pattern()?;

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		let (_, mut end) = expect_token_and_advance!(self, Token::RightBrace);

		// Optional `else { ... }` or `else if ...`, allowing line breaks between
		// `}` and `else`.
		let else_body = if matches!(self.peek_past_breaks(), Some(Token::KeywordElse(..))) {
			self.skip_line_breaks();
			self.advance();
			if matches!(self.current_token, Some(Token::KeywordIf(..))) {
				// `else if ...` — the chained `if` is the sole else expression,
				// parsed without braces so chains stay flat rather than nesting a
				// fresh `else { if ... }` block (and its closing brace) per arm.
				let if_node = self.parse_if_expression()?;
				end = if_node.range.end;
				Some(vec![ExprNode {
					range: if_node.range,
					kind: ExprKind::If(if_node),
					ty: Type::Unknown,
					trait_dispatch: None,
					dispatch_sink: None,
				}])
			} else {
				expect_token_and_advance!(self, Token::LeftBrace);
				let else_body = self.parse_body_expressions()?;
				let (_, else_end) = expect_token_and_advance!(self, Token::RightBrace);
				end = else_end;
				Some(else_body)
			}
		} else {
			None
		};

		Some(IfNode {
			range: Range::between(start, end),
			subject: Box::new(condition),
			pattern,
			body,
			else_body,
		})
	}

	fn parse_when_expression(&mut self) -> Option<WhenNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordWhen);

		let subject = self.parse_expression()?;

		self.skip_line_breaks();

		let mut cases = Vec::new();

		loop {
			let case_start = self.offset_to_point(self.current_token_span().0);

			// `else { body }` is sugar for `is _ { body }`. Once we see it,
			// the case list ends — no more `is` arms can follow.
			let (case_pattern, is_else) = match self.current_token {
				Some(Token::KeywordIs(..)) => {
					self.advance();
					(self.parse_pattern()?, false)
				}
				Some(Token::KeywordElse(start, end)) => {
					self.advance();
					let p_start = self.offset_to_point(start);
					let p_end = self.offset_to_point(end);
					(
						PatternNode {
							range: Range::between(p_start, p_end),
							kind: PatternKind::Underscore,
						},
						true,
					)
				}
				_ => break,
			};

			expect_token_and_advance!(self, Token::LeftBrace);

			let case_body = self.parse_body_expressions()?;

			self.skip_line_breaks();

			let (_, case_end) = expect_token_and_advance!(self, Token::RightBrace);

			cases.push(CaseNode {
				range: Range::between(case_start, case_end),
				pattern: case_pattern,
				body: case_body,
			});

			if is_else {
				break;
			}

			// Only consume trailing breaks if another `is`/`else` case follows.
			// Without this, breaks after the final `}` get eaten and whatever
			// comes next (e.g. another `when` statement) gets parsed as a
			// function-call arg.
			if matches!(
				self.peek_past_breaks(),
				Some(Token::KeywordIs(..)) | Some(Token::KeywordElse(..))
			) {
				self.skip_line_breaks();
			}
		}

		// TODO: error if 0 cases
		let end = cases.last().expect("> 0 cases").range.end;

		Some(WhenNode {
			range: Range::between(start, end),
			subject: Box::new(subject),
			cases,
		})
	}

	fn parse_while_expression(&mut self) -> Option<WhileNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordWhile);

		let subject = self.parse_expression()?;

		expect_token_and_advance!(self, Token::KeywordIs);

		let pattern = self.parse_pattern()?;

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(WhileNode {
			range: Range::between(start, end),
			subject: Box::new(subject),
			pattern,
			body,
		})
	}

	// `scope (as IDENT)? { body }` or `manual scope as IDENT { body }`. The
	// `manual` prefix (if present) is consumed first; the body parses like any
	// block body, so `try`/`let`/`defer` work inside.
	fn parse_scope_expression(&mut self) -> Option<ScopeNode> {
		let manual_start = if let Some(Token::KeywordManual(s, _)) = self.current_token {
			self.advance();
			Some(self.offset_to_point(s))
		} else {
			None
		};
		let manual = manual_start.is_some();

		let (scope_start, _) = expect_token_and_advance!(self, Token::KeywordScope);
		let start = manual_start.unwrap_or(scope_start);

		let handle = if matches!(self.current_token, Some(Token::KeywordAs(..))) {
			self.advance();
			Some(self.expect_identifier()?)
		} else {
			None
		};

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(ScopeNode {
			range: Range::between(start, end),
			manual,
			handle,
			body,
		})
	}

	fn parse_pattern(&mut self) -> Option<PatternNode> {
		match self.current_token {
			Some(Token::Identifier(..)) => {
				let id_node = self.parse_identifier().unwrap();

				let mut args = Vec::new();
				while let Some(arg) = self.parse_pattern_atom() {
					args.push(arg);
				}

				if !args.is_empty() {
					let end = args.last().unwrap().range.end;
					return Some(PatternNode {
						range: Range::between(id_node.range.start, end),
						kind: PatternKind::Constructor(id_node, args),
					});
				}

				Some(PatternNode {
					range: id_node.range,
					kind: PatternKind::Identifier(id_node),
				})
			}

			Some(Token::LeftParen(..)) => self.parse_paren_pattern(),

			Some(Token::LeftBracket(..)) => self.parse_list_pattern(),

			Some(Token::LeftBrace(..)) => self.parse_record_pattern(),

			Some(Token::Underscore(start, end)) => {
				self.advance();

				Some(PatternNode {
					range: self.span_to_single_line_range(start, end),
					kind: PatternKind::Underscore,
				})
			}

			Some(Token::StringLiteral(..)) => self.parse_string().map(|expr_node| match expr_node.kind {
				ExprKind::Literal(literal) => PatternNode {
					range: literal.range,
					kind: PatternKind::Literal(literal),
				},
				ExprKind::Interpolation(parts) => PatternNode {
					range: expr_node.range,
					kind: PatternKind::Interpolation(parts),
				},
				_ => unreachable!(),
			}),

			Some(Token::BytesLiteral(..)) => self.parse_bytes().map(|expr_node| match expr_node.kind {
				ExprKind::Literal(literal) => PatternNode {
					range: literal.range,
					kind: PatternKind::Literal(literal),
				},
				_ => unreachable!(),
			}),

			Some(Token::BoolFalse(..) | Token::BoolTrue(..)) => {
				let expr_node = self.parse_bool()?;
				if let ExprKind::Literal(lit_node) = expr_node.kind {
					Some(PatternNode {
						range: expr_node.range,
						kind: PatternKind::Literal(lit_node),
					})
				} else {
					None
				}
			}

			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| PatternNode {
				range: lit_node.range,
				kind: PatternKind::Literal(lit_node),
			}),

			Some(Token::DurationLiteral(..)) => {
				self.parse_duration_literal().map(|lit_node| PatternNode {
					range: lit_node.range,
					kind: PatternKind::Literal(lit_node),
				})
			}

			// TODO: other kinds of digits here
			_ => None,
		}
	}

	// Parse `(...)` in pattern position. A single inner pattern with no comma is
	// treated as grouping (returned directly); otherwise it's a Tuple pattern.
	fn parse_paren_pattern(&mut self) -> Option<PatternNode> {
		let start_offset = match self.current_token {
			Some(Token::LeftParen(s, _)) => s,
			_ => return None,
		};
		let start = self.offset_to_point(start_offset);

		self.advance();
		self.skip_line_breaks();

		// `()` — empty tuple pattern
		if matches!(self.current_token, Some(Token::RightParen(..))) {
			let (_, end) = expect_token_and_advance!(self, Token::RightParen);
			return Some(PatternNode {
				range: Range::between(start, end),
				kind: PatternKind::Tuple(vec![]),
			});
		}

		let first = self.parse_pattern()?;
		self.skip_line_breaks();

		match self.current_token {
			Some(Token::Comma(..)) => {
				self.advance();
				self.skip_line_breaks();

				let mut entries = vec![first];
				while let Some(p) = self.parse_pattern() {
					entries.push(p);
					match self.current_token {
						Some(Token::Comma(..)) => {
							self.advance();
							self.skip_line_breaks();
						}
						_ => break,
					}
				}

				self.skip_line_breaks();
				let (_, end) = expect_token_and_advance!(self, Token::RightParen);

				Some(PatternNode {
					range: Range::between(start, end),
					kind: PatternKind::Tuple(entries),
				})
			}
			_ => {
				expect_token_and_advance!(self, Token::RightParen);
				Some(first)
			}
		}
	}

	// Parse `{` ... `}` in pattern position. Supports:
	//   {}                  — closed empty record
	//   {a: p, b: q}        — closed: subject must have exactly these fields
	//   {a: p, ...}         — open, no rest binding
	//   {a: p, ...rest}     — open, bind the remaining fields as `rest`
	//   {...}               — open empty
	//   {...rest}           — open empty, bind whole record as `rest`
	fn parse_record_pattern(&mut self) -> Option<PatternNode> {
		let (start_offset, _) = match self.current_token {
			Some(Token::LeftBrace(s, e)) => (s, e),
			_ => return None,
		};
		let start = self.offset_to_point(start_offset);

		self.advance();
		self.skip_line_breaks();

		let mut fields = Vec::new();
		let mut rest: Option<RecordRestPattern> = None;

		if let Some(Token::TripleDot(..)) = self.current_token {
			rest = Some(self.parse_record_rest_pattern()?);
			self.skip_line_breaks();
			let (_, end) = expect_token_and_advance!(self, Token::RightBrace);
			return Some(PatternNode {
				range: Range::between(start, end),
				kind: PatternKind::Record { fields, rest },
			});
		}

		while let Some(field_name) = self.parse_identifier() {
			// Field shorthand: `{a, b}` desugars to `{a: a, b: b}`. The
			// sub-pattern is an identifier pattern binding the same name.
			let field_pattern = if matches!(self.current_token, Some(Token::Colon(..))) {
				self.advance();
				self.parse_pattern()?
			} else {
				PatternNode {
					range: field_name.range,
					kind: PatternKind::Identifier(field_name.clone()),
				}
			};

			fields.push((field_name, field_pattern));

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
					if let Some(Token::TripleDot(..)) = self.current_token {
						rest = Some(self.parse_record_rest_pattern()?);
						self.skip_line_breaks();
						break;
					}
				}
				Some(Token::LineBreak(..)) => {
					self.skip_line_breaks();
				}
				_ => break,
			}
		}

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);
		Some(PatternNode {
			range: Range::between(start, end),
			kind: PatternKind::Record { fields, rest },
		})
	}

	// Parse `[` ... `]` in pattern position. Supports:
	//   []                  — empty list
	//   [a, b, c]           — exact length
	//   [a, b, ...]         — at least 2 elements, no rest binding
	//   [a, b, ...rest]     — at least 2 elements, bind tail as `list a`
	//   [...]               — any list (no items)
	//   [...rest]           — any list, bind whole thing as the rest
	fn parse_list_pattern(&mut self) -> Option<PatternNode> {
		let (start, _) = expect_token_and_advance!(self, Token::LeftBracket);
		self.skip_line_breaks();

		let mut items = Vec::new();
		let mut rest: Option<ListRestPattern> = None;

		// Allow `...` at the very start: `[...rest]` or `[...]`.
		if let Some(Token::TripleDot(..)) = self.current_token {
			rest = Some(self.parse_list_rest_pattern()?);
			self.skip_line_breaks();
			let (_, end) = expect_token_and_advance!(self, Token::RightBracket);
			return Some(PatternNode {
				range: Range::between(start, end),
				kind: PatternKind::List { items, rest },
			});
		}

		// Empty list `[]`.
		if let Some(Token::RightBracket(..)) = self.current_token {
			let (_, end) = expect_token_and_advance!(self, Token::RightBracket);
			return Some(PatternNode {
				range: Range::between(start, end),
				kind: PatternKind::List { items, rest },
			});
		}

		loop {
			let item = self.parse_pattern()?;
			items.push(item);
			self.skip_line_breaks();

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
					// After a comma, the next thing might be a `...rest` —
					// it can only appear in the trailing position.
					if let Some(Token::TripleDot(..)) = self.current_token {
						rest = Some(self.parse_list_rest_pattern()?);
						self.skip_line_breaks();
						break;
					}
				}
				_ => break,
			}
		}

		let (_, end) = expect_token_and_advance!(self, Token::RightBracket);
		Some(PatternNode {
			range: Range::between(start, end),
			kind: PatternKind::List { items, rest },
		})
	}

	fn parse_list_rest_pattern(&mut self) -> Option<ListRestPattern> {
		let (start, dot_end) = expect_token_and_advance!(self, Token::TripleDot);
		// Optional identifier directly after `...`.
		if let Some(Token::Identifier(..)) = self.current_token {
			let ident = self.parse_identifier()?;
			let range = Range::between(start, ident.range.end);
			return Some(ListRestPattern {
				range,
				binding: Some(ident),
			});
		}
		Some(ListRestPattern {
			range: Range::between(start, dot_end),
			binding: None,
		})
	}

	fn parse_record_rest_pattern(&mut self) -> Option<RecordRestPattern> {
		let (start, dot_end) = expect_token_and_advance!(self, Token::TripleDot);
		if let Some(Token::Identifier(..)) = self.current_token {
			let ident = self.parse_identifier()?;
			let range = Range::between(start, ident.range.end);
			return Some(RecordRestPattern {
				range,
				binding: Some(ident),
			});
		}
		Some(RecordRestPattern {
			range: Range::between(start, dot_end),
			binding: None,
		})
	}

	// A sub-pattern that does not itself try to consume constructor arguments,
	// used when parsing the args of a Constructor pattern. Without this, every
	// arg ident would greedily try to become its own Constructor.
	fn parse_pattern_atom(&mut self) -> Option<PatternNode> {
		match self.current_token {
			Some(Token::Identifier(..)) => {
				let id_node = self.parse_identifier().unwrap();
				Some(PatternNode {
					range: id_node.range,
					kind: PatternKind::Identifier(id_node),
				})
			}

			// Parens let nested constructor patterns appear as constructor args:
			// `some (node val l r)` becomes Constructor(some, [Constructor(node, [...])])
			// rather than the flat Constructor(some, [node, val, l, r]).
			Some(Token::LeftParen(..)) => self.parse_paren_pattern(),

			Some(Token::LeftBracket(..)) => self.parse_list_pattern(),

			// Not LeftBrace: a `{` that immediately follows a constructor head
			// (`some {a: x}`) is ambiguous with the case body's `{`, so record
			// patterns in that position must be wrapped in parens
			// (`some ({a: x})`). The paren branch above handles that.
			Some(Token::Underscore(start, end)) => {
				self.advance();
				Some(PatternNode {
					range: self.span_to_single_line_range(start, end),
					kind: PatternKind::Underscore,
				})
			}

			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| PatternNode {
				range: lit_node.range,
				kind: PatternKind::Literal(lit_node),
			}),

			Some(Token::DurationLiteral(..)) => {
				self.parse_duration_literal().map(|lit_node| PatternNode {
					range: lit_node.range,
					kind: PatternKind::Literal(lit_node),
				})
			}

			Some(Token::BoolFalse(..) | Token::BoolTrue(..)) => {
				let expr_node = self.parse_bool()?;
				if let ExprKind::Literal(lit_node) = expr_node.kind {
					Some(PatternNode {
						range: expr_node.range,
						kind: PatternKind::Literal(lit_node),
					})
				} else {
					None
				}
			}

			Some(Token::StringLiteral(..)) => self.parse_string().map(|expr_node| match expr_node.kind {
				ExprKind::Literal(literal) => PatternNode {
					range: literal.range,
					kind: PatternKind::Literal(literal),
				},
				ExprKind::Interpolation(parts) => PatternNode {
					range: expr_node.range,
					kind: PatternKind::Interpolation(parts),
				},
				_ => unreachable!(),
			}),

			Some(Token::BytesLiteral(..)) => self.parse_bytes().map(|expr_node| match expr_node.kind {
				ExprKind::Literal(literal) => PatternNode {
					range: literal.range,
					kind: PatternKind::Literal(literal),
				},
				_ => unreachable!(),
			}),

			_ => None,
		}
	}

	// `defer Expr` — a body statement that schedules `Expr` to run when the
	// enclosing function exits. Like `let`, it greedily consumes a full
	// expression for its operand and evaluates to `nothing`.
	fn parse_defer(&mut self) -> Option<ExprNode> {
		let (start, end) = expect_token_and_advance!(self, Token::KeywordDefer);
		let Some(inner) = self.parse_expression() else {
			// `defer` with no operand (e.g. `defer` then end of block). Report
			// rather than silently truncating the enclosing block.
			return self.error(ParseError {
				range: Range::between(start, end),
				kind: ParseErrorKind::ExpectedExpressionAfterDefer,
			});
		};
		let range = Range::between(start, inner.range.end);
		Some(ExprNode {
			range,
			kind: ExprKind::Defer(Box::new(inner)),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_let_expression(&mut self) -> Option<LetNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordLet);

		let pattern = match self.parse_pattern() {
			Some(p) => p,
			None => {
				// Surface the same "expected an identifier" diagnostic users
				// got before destructuring patterns landed — most of the
				// time the LHS is just an identifier, and that's the most
				// likely thing they got wrong.
				self.expect_identifier()?;
				return None;
			}
		};

		// `:: TYPE` annotation — same shape as the top-level def form.
		// Only meaningful on identifier patterns; the analyzer enforces
		// that and surfaces a diagnostic if it appears alongside a
		// destructuring pattern.
		let type_annotation = if matches!(self.current_token, Some(Token::DoubleColon(..))) {
			self.advance();
			self.skip_line_breaks();
			Some(self.parse_type_expression_with_generics()?)
		} else {
			None
		};

		expect_token_and_advance!(self, Token::Equal);

		let (end, value) = match self.parse_expression() {
			Some(node) => (node.range.end, node),
			_ => {
				// if we failed to parse this expression, we've already reported
				// an error about it, so just return nothing here
				return None;
			}
		};

		Some(LetNode {
			range: Range::between(start, end),
			pattern,
			value: Box::new(value),
			type_annotation,
		})
	}

	fn parse_list(&mut self) -> Option<ExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::LeftBracket);

		self.skip_line_breaks();

		let mut elements = Vec::new();

		loop {
			// A leading `...` makes this element a spread (must be a list).
			// Unlike list *patterns*, spreads may appear at any position and
			// any number of times.
			let spread = if let Some(Token::TripleDot(span_start, span_end)) = self.current_token {
				self.advance();
				Some((span_start, span_end))
			} else {
				None
			};

			let Some(expr) = self.parse_expression() else {
				if let Some((span_start, span_end)) = spread {
					// `...` with nothing after it (e.g. `[...]` or `[1, ...]`).
					self.error::<ExprNode>(ParseError {
						range: Range::between(
							self.offset_to_point(span_start),
							self.offset_to_point(span_end),
						),
						kind: ParseErrorKind::ExpectedExpressionAfterSpread,
					});
				}
				break;
			};

			elements.push(if spread.is_some() {
				ListItem::Spread(expr)
			} else {
				ListItem::Item(expr)
			});

			if current_token_is!(self, Token::Comma) {
				self.advance();
				self.skip_line_breaks();
			} else {
				break;
			}
		}

		self.skip_line_breaks();

		let (_, end) = expect_token_and_advance!(self, Token::RightBracket);

		Some(ExprNode {
			range: Range::between(start, end),
			kind: ExprKind::List(elements),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_numeric_literal(
		&mut self,
		start_offset: usize,
		end_offset: usize,
		radix: usize,
	) -> usize {
		let mut result: usize = 0;
		let mut i: usize = 1;

		for byte in self.source[start_offset..end_offset].iter().rev() {
			let byte_value = match byte {
				b'0'..=b'9' => byte - 48,
				b'A'..=b'F' => byte - 55,
				b'a'..=b'f' => byte - 87,
				_ => unreachable!(),
			} as usize;

			if let Some(next_result) = result.checked_add(byte_value * i) {
				result = next_result;
			} else {
				self.error::<LiteralNode>(ParseError {
					range: Range::between(
						self.offset_to_point(start_offset),
						self.offset_to_point(end_offset),
					),
					kind: ParseErrorKind::OverflowingIntegerLiteral,
				});
				return 0;
			}

			if let Some(next_i) = i.checked_mul(radix) {
				i = next_i;
			} else {
				self.error::<LiteralNode>(ParseError {
					range: Range::between(
						self.offset_to_point(start_offset),
						self.offset_to_point(end_offset),
					),
					kind: ParseErrorKind::OverflowingIntegerLiteral,
				});
				return 0;
			}
		}

		result
	}

	fn parse_record(&mut self) -> Option<ExprNode> {
		let (record_start, _) = expect_token_and_advance!(self, Token::LeftBrace);

		self.skip_line_breaks();

		let mut entries = Vec::new();

		while let Some(field_name) = self.parse_identifier() {
			// Field shorthand: `{a, b}` desugars to `{a: a, b: b}`. The
			// value is the same identifier resolved from the surrounding
			// scope.
			let field_value = if matches!(self.current_token, Some(Token::Colon(..))) {
				self.advance();
				self.parse_expression()?
			} else {
				ExprNode {
					range: field_name.range,
					kind: ExprKind::Identifier(field_name.clone()),
					ty: Type::Unknown,
					trait_dispatch: None,
					dispatch_sink: None,
				}
			};

			entries.push((field_name, field_value));

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
				}
				Some(Token::LineBreak(..)) => {
					self.skip_line_breaks();
				}
				_ => break,
			}
		}

		self.skip_line_breaks();

		let (_, record_end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(ExprNode {
			range: Range::between(record_start, record_end),
			kind: ExprKind::Record(entries),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_parenthetical(&mut self) -> Option<ExprNode> {
		// "parentheticals" could be a number of things:
		//  - "()" is an empty tuple
		//  - "(expr)" is an expression in parentheses (a grouping),
		//  - "(expr1, expr2, expr3)" is a tuple

		let (paren_start, _) = expect_token_and_advance!(self, Token::LeftParen);

		self.skip_line_breaks();

		let mut entries = Vec::new();

		while let Some(node) = self.parse_expression() {
			entries.push(node);

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
				}
				Some(Token::LineBreak(..)) => {
					self.skip_line_breaks();
				}
				_ => break,
			}
		}

		self.skip_line_breaks();

		let (_, paren_end) = expect_token_and_advance!(self, Token::RightParen);

		if entries.is_empty() {
			return Some(ExprNode {
				range: Range::between(paren_start, paren_end),
				kind: ExprKind::EmptyTuple,
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			});
		}

		if entries.len() == 1 {
			// If only one expression was found, it's a grouping
			if let Some(first_expr) = entries.pop() {
				return Some(ExprNode {
					range: Range::between(paren_start, paren_end),
					kind: ExprKind::Grouping(Box::new(first_expr)),
					ty: Type::Unknown,
					trait_dispatch: None,
					dispatch_sink: None,
				});
			}
		}

		// Otherwise, it's a tuple with multiple entries:
		Some(ExprNode {
			range: Range::between(paren_start, paren_end),
			kind: ExprKind::Tuple(entries),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_regular_expression(&mut self) -> Option<ExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::Backtick);

		self.skip_line_breaks();

		let maybe_reg_expr_node = self.parse_regular_expression_body();

		self.skip_line_breaks();

		let (_, end) = expect_token_and_advance!(self, Token::Backtick);

		let regex = match maybe_reg_expr_node {
			Some(expr) => expr,
			None => {
				return self.error(ParseError {
					range: Range::between(start, end),
					kind: ParseErrorKind::EmptyRegularExpression,
				})
			}
		};

		Some(ExprNode {
			range: Range::between(start, end),
			kind: ExprKind::Regex(regex),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_regular_expression_body(&mut self) -> Option<RegexNode> {
		let mut first_term = None;
		let mut other_terms = Vec::new();

		let mut term = self.parse_regular_expression_term();

		loop {
			if term.is_some() {
				self.skip_line_breaks();

				if first_term.is_none() {
					first_term = term;
				} else {
					other_terms.push(term.unwrap());
				}

				match self.current_token {
					Some(Token::Pipe(..)) => {
						self.advance();
						term = self.parse_regular_expression_term();
						continue;
					}
					_ => {}
				}
			}

			break;
		}

		if first_term.is_none() {
			return None;
		}

		if other_terms.is_empty() {
			return first_term;
		}

		other_terms.insert(0, first_term.unwrap());

		let start = other_terms.first().unwrap().range.start;
		let end = other_terms.last().unwrap().range.end;

		Some(RegexNode {
			range: Range::between(start, end),
			kind: RegexKind::Alternation(other_terms),
		})
	}

	fn parse_regular_expression_term(&mut self) -> Option<RegexNode> {
		let mut first_part = None;
		let mut other_parts = Vec::new();

		loop {
			self.skip_line_breaks();

			let (part, part_is_anchor) = match self.current_token {
				Some(Token::Caret(start_offset, end_offset)) => {
					self.advance();
					(
						RegexNode {
							range: self.span_to_single_line_range(start_offset, end_offset),
							kind: RegexKind::Anchor(RegexAnchor::Start),
						},
						true,
					)
				}

				Some(Token::Dollar(start_offset, end_offset)) => {
					self.advance();
					(
						RegexNode {
							range: self.span_to_single_line_range(start_offset, end_offset),
							kind: RegexKind::Anchor(RegexAnchor::End),
						},
						true,
					)
				}

				Some(Token::Percent(start_offset, end_offset)) => {
					self.advance();
					(
						RegexNode {
							range: self.span_to_single_line_range(start_offset, end_offset),
							kind: RegexKind::Anchor(RegexAnchor::Boundary),
						},
						true,
					)
				}

				Some(Token::Identifier(start_offset, end_offset)) => {
					self.advance();

					let name = read_string!(self, start_offset, end_offset);

					(
						RegexNode {
							range: self.span_to_single_line_range(start_offset, end_offset),
							kind: RegexKind::CharacterClass(name),
						},
						false,
					)
				}

				Some(Token::StringLiteral(start_offset, end_offset)) => {
					self.advance();

					let value = read_string_with_escapes!(self, start_offset, end_offset);

					(
						RegexNode {
							range: self.span_to_single_line_range(start_offset, end_offset),
							kind: RegexKind::Literal(value),
						},
						false,
					)
				}

				Some(Token::LeftParen(start_offset, end_offset)) => {
					let start = self.offset_to_point(start_offset);

					self.advance();

					let expr = match self.parse_regular_expression_body() {
						Some(expr) => expr,
						None => {
							return self.error(ParseError {
								range: self.span_to_single_line_range(start_offset, end_offset),
								kind: ParseErrorKind::EmptyRegularExpressionGroup,
							})
						}
					};

					let (_, end) = expect_token_and_advance!(self, Token::RightParen);

					(
						RegexNode {
							range: Range::between(start, end),
							kind: RegexKind::Grouping(Box::new(expr)),
						},
						false,
					)
				}

				Some(Token::LeftAngle(start_offset, end_offset)) => {
					let start = self.offset_to_point(start_offset);
					let end = self.offset_to_point(end_offset);

					self.advance();

					let (name_start, name_end) = expect_token_and_advance!(self, Token::Identifier);
					let name = read_string!(
						self,
						self.point_to_offset(name_start),
						self.point_to_offset(name_end)
					);

					expect_token_and_advance!(self, Token::Colon);

					let expr = match self.parse_regular_expression_body() {
						Some(expr) => expr,
						None => {
							return self.error(ParseError {
								range: Range::between(start, end),
								kind: ParseErrorKind::EmptyRegularExpressionGroup,
							})
						}
					};

					expect_token_and_advance!(self, Token::RightAngle);

					(
						RegexNode {
							range: Range::between(start, end),
							kind: RegexKind::NamedCapture(name, Box::new(expr)),
						},
						false,
					)
				}

				_ => break,
			};

			// Anchors are zero-width, so quantifying them doesn't make sense.
			// Surface that as a parse error rather than passing through to a
			// confusing error from the underlying regex engine.
			if part_is_anchor {
				if let Some(
					Token::Star(..) | Token::Plus(..) | Token::Question(..) | Token::LeftBrace(..),
				) = self.current_token
				{
					let (q_start, q_end) = self.current_token_span();
					self.error::<RegexNode>(ParseError {
						range: self.span_to_single_line_range(q_start, q_end),
						kind: ParseErrorKind::QuantifierOnRegexAnchor,
					});
					self.advance();
				}

				self.skip_line_breaks();
				if first_part.is_none() {
					first_part = Some(part);
				} else {
					other_parts.push(part);
				}
				continue;
			}

			let modified_part = match self.current_token {
				Some(Token::Star(_, end_offset)) => {
					self.advance();

					RegexNode {
						range: Range::between(part.range.start, self.offset_to_point(end_offset)),
						kind: RegexKind::ZeroOrMore(Box::new(part)),
					}
				}

				Some(Token::Plus(_, end_offset)) => {
					self.advance();

					RegexNode {
						range: Range::between(part.range.start, self.offset_to_point(end_offset)),
						kind: RegexKind::OneOrMore(Box::new(part)),
					}
				}

				Some(Token::Question(_, end_offset)) => {
					self.advance();

					RegexNode {
						range: Range::between(part.range.start, self.offset_to_point(end_offset)),
						kind: RegexKind::OneOrZero(Box::new(part)),
					}
				}

				Some(Token::LeftBrace(_, _)) => {
					self.advance();

					let mut min_count = None;
					let mut max_count = None;
					let mut has_comma = false;

					if current_token_is!(self, Token::DecimalDigits) {
						let (start, end) = self.current_token_span();
						let value = self.parse_numeric_literal(start, end, 10) as usize;
						min_count = Some(value);
						self.advance();
					}

					if current_token_is!(self, Token::Comma) {
						has_comma = true;

						self.advance();

						if current_token_is!(self, Token::DecimalDigits) {
							let (start, end) = self.current_token_span();
							let value = self.parse_numeric_literal(start, end, 10) as usize;
							max_count = Some(value);
							self.advance();
						}
					}

					let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

					match (min_count, max_count, has_comma) {
						(Some(min), None, true) => RegexNode {
							range: Range::between(part.range.start, end),
							kind: RegexKind::AtLeastCount(Box::new(part), min),
						},

						(None, Some(max), true) => RegexNode {
							range: Range::between(part.range.start, end),
							kind: RegexKind::AtMostCount(Box::new(part), max),
						},

						(Some(min), None, false) => RegexNode {
							range: Range::between(part.range.start, end),
							kind: RegexKind::ExactCount(Box::new(part), min),
						},

						(Some(min), Some(max), true) => {
							if min > max {
								self.error::<RegexNode>(ParseError {
									range: Range::between(part.range.start, end),
									kind: ParseErrorKind::InvalidRegularExpressionCountModifier,
								});
							}

							RegexNode {
								range: Range::between(part.range.start, end),
								kind: RegexKind::RangeCount(Box::new(part), min, max),
							}
						}

						_ => {
							return self.error(ParseError {
								range: Range::between(part.range.start, end),
								kind: ParseErrorKind::EmptyRegularExpressionCount,
							})
						}
					}
				}

				_ => part,
			};

			self.skip_line_breaks();

			if first_part.is_none() {
				first_part = Some(modified_part);
			} else {
				other_parts.push(modified_part);
			}
		}

		if other_parts.is_empty() {
			if first_part.is_some() {
				return first_part;
			}

			return None;
		}

		match first_part {
			Some(part) => other_parts.insert(0, part),
			None => return None,
		};

		let start = other_parts.first().unwrap().range.start;
		let end = other_parts.last().unwrap().range.end;

		Some(RegexNode {
			range: Range::between(start, end),
			kind: RegexKind::Sequence(other_parts),
		})
	}

	fn parse_definition(&mut self) -> Option<DefinitionNode> {
		// Optional leading visibility modifier. `public` exports a def
		// fully; `opaque` exports an enum's type but hides its
		// constructors. Absent means private. The modifier's span becomes
		// the definition's range start so the keyword is included.
		let (visibility, modifier_span) = match self.current_token {
			Some(Token::KeywordPublic(s, e)) => {
				self.advance();
				(
					Visibility::Public,
					Some((self.offset_to_point(s), self.offset_to_point(e))),
				)
			}
			Some(Token::KeywordOpaque(s, e)) => {
				self.advance();
				(
					Visibility::Opaque,
					Some((self.offset_to_point(s), self.offset_to_point(e))),
				)
			}
			_ => (Visibility::Private, None),
		};
		let mod_start = modifier_span.map(|(s, _)| s);

		// `public`/`opaque` only modify a `def`, `enum`, or `alias`, and
		// `opaque` only an `enum`. Reject anything else (instances, traits,
		// a dangling modifier) up front.
		if visibility != Visibility::Private {
			let target_ok = match self.current_token {
				Some(Token::KeywordEnum(..)) => true,
				Some(Token::KeywordAlias(..)) | Some(Token::KeywordDef(..)) => {
					visibility != Visibility::Opaque
				}
				_ => false,
			};
			if !target_ok {
				let (start, end) = modifier_span.unwrap();
				let keyword = if visibility == Visibility::Opaque {
					"opaque"
				} else {
					"public"
				};
				return self.error(ParseError {
					range: Range::between(start, end),
					kind: ParseErrorKind::MisplacedVisibility { keyword },
				});
			}
		}

		// Instance: `implement TRAIT TYPE [where ...] { defs }`.
		if let Some(Token::KeywordImplement(start_offset, _)) = self.current_token {
			let start = self.offset_to_point(start_offset);
			self.advance();
			return self.parse_instance_after_implement(start);
		}

		// `enum NAME [PARAMS] { variants }` — top-level enum type.
		if let Some(Token::KeywordEnum(start_offset, _)) = self.current_token {
			let start = mod_start.unwrap_or(self.offset_to_point(start_offset));
			self.advance();
			let name = self.expect_identifier()?;
			let enum_node = self.parse_enum()?;
			self.skip_line_breaks();
			return Some(DefinitionNode {
				name,
				range: Range::between(start, enum_node.range.end),
				kind: DefinitionKind::Enum(enum_node),
				visibility,
				ty: Type::Unknown,
				dict_param_count: 0,
				type_annotation: None,
				where_clause: Vec::new(),
			});
		}

		// `alias NAME TYPE_EXPR` — top-level alias type.
		if let Some(Token::KeywordAlias(start_offset, _)) = self.current_token {
			let start = mod_start.unwrap_or(self.offset_to_point(start_offset));
			self.advance();
			let name = self.expect_identifier()?;
			let type_expr = self.parse_type_expression_with_generics()?;
			self.skip_line_breaks();
			return Some(DefinitionNode {
				name,
				range: Range::between(start, type_expr.range.end),
				kind: DefinitionKind::Alias(type_expr),
				visibility,
				ty: Type::Unknown,
				dict_param_count: 0,
				type_annotation: None,
				where_clause: Vec::new(),
			});
		}

		// `trait NAME PARAM { methods }` — top-level trait declaration.
		if let Some(Token::KeywordTrait(start_offset, _)) = self.current_token {
			let start = self.offset_to_point(start_offset);
			self.advance();
			let name = self.expect_identifier()?;
			let trait_node = self.parse_trait()?;
			self.skip_line_breaks();
			return Some(DefinitionNode {
				name,
				range: Range::between(start, trait_node.range.end),
				kind: DefinitionKind::Trait(trait_node),
				// Traits aren't subject to the visibility ladder yet; the
				// guard above guarantees `visibility` is `Private` here.
				visibility,
				ty: Type::Unknown,
				dict_param_count: 0,
				type_annotation: None,
				where_clause: Vec::new(),
			});
		}

		// `def NAME [:: TYPE] = EXPR` — value binding. The optional
		// `:: TYPE` annotation is the contract; the analyzer unifies
		// the inferred body type with the annotated type.
		let start = match self.current_token {
			Some(Token::KeywordDef(start_offset, _)) => {
				let point = mod_start.unwrap_or(self.offset_to_point(start_offset));
				self.advance();
				point
			}
			_ => return None,
		};

		let name = self.expect_identifier()?;

		let type_annotation = if matches!(self.current_token, Some(Token::DoubleColon(..))) {
			self.advance();
			self.skip_line_breaks();
			Some(self.parse_type_expression_with_generics()?)
		} else {
			None
		};

		self.skip_line_breaks();

		// Optional `where (trait param, ...)` clause on the signature.
		let where_clause = self.parse_where_clause()?;

		self.skip_line_breaks();

		expect_token_and_advance!(self, Token::Equal);

		// Allow the RHS to wrap to the next line — long signatures
		// (especially with `built-in "..."` tails) read more clearly
		// when the body starts on a fresh, indented line.
		self.skip_line_breaks();

		let value = self.parse_expression()?;

		self.skip_line_breaks();

		Some(DefinitionNode {
			name,
			range: Range::between(start, value.range.end),
			kind: DefinitionKind::Expr(value),
			visibility,
			ty: Type::Unknown,
			dict_param_count: 0,
			type_annotation,
			where_clause,
		})
	}

	// Trait body: `trait NAME PARAM { method-sigs / defaults }`. The
	// `trait NAME` prefix has already been consumed by the caller.
	//
	// Method signature: `METHOD_NAME :: TYPE_EXPR`. The type expression is
	// usually a function type, but parsing accepts any type expression —
	// the analyzer rejects non-function signatures later.
	//
	// Default body: `def METHOD_NAME = EXPR`. Stored as an `ExprNode` on
	// the matching method.
	fn parse_trait(&mut self) -> Option<TraitNode> {
		// Required single type parameter (`a` in `trait numeric a { ... }`).
		let param = self.expect_identifier()?;

		let (brace_start, _) = expect_token_and_advance!(self, Token::LeftBrace);

		self.skip_line_breaks();

		let mut methods: Vec<TraitMethodNode> = Vec::new();

		loop {
			self.skip_line_breaks();

			// `def METHOD = EXPR`: attach a default body to a previously
			// declared signature with the same name.
			if let Some(Token::KeywordDef(start_offset, _)) = self.current_token {
				let default_start = self.offset_to_point(start_offset);
				self.advance();

				let method_name = self.expect_identifier()?;
				expect_token_and_advance!(self, Token::Equal);
				let body = self.parse_expression()?;

				// Find the matching signature; default without a signature is
				// a parse error.
				match methods.iter_mut().find(|m| m.name.name == method_name.name) {
					Some(m) => {
						m.range = Range::between(m.range.start, body.range.end);
						m.default = Some(body);
					}
					None => {
						return self.error(ParseError {
							range: Range::between(default_start, method_name.range.end),
							kind: ParseErrorKind::InvalidDefBody,
						});
					}
				}

				self.skip_line_breaks();
				continue;
			}

			// Method signature: `NAME :: TYPE_EXPR`.
			if matches!(self.current_token, Some(Token::Identifier(..))) {
				let method_name = self.parse_identifier()?;
				expect_token_and_advance!(self, Token::DoubleColon);
				let signature = self.parse_type_expression_with_generics()?;

				methods.push(TraitMethodNode {
					range: Range::between(method_name.range.start, signature.range.end),
					name: method_name,
					signature,
					default: None,
				});

				self.skip_line_breaks();
				continue;
			}

			break;
		}

		let (_, brace_end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(TraitNode {
			range: Range::between(brace_start, brace_end),
			param,
			methods,
		})
	}

	// Optional `where (TRAIT PARAM, TRAIT PARAM, ...)` clause. Returns an
	// empty vec when no `where` keyword is present. Shared by instance
	// heads (`implement TRAIT TYPE where (...)`) and top-level def
	// signatures (`def name :: TYPE where (...) = ...`).
	fn parse_where_clause(&mut self) -> Option<Vec<InstanceConstraintNode>> {
		if !matches!(self.current_token, Some(Token::KeywordWhere(..))) {
			return Some(Vec::new());
		}
		self.advance();
		expect_token_and_advance!(self, Token::LeftParen);
		self.skip_line_breaks();

		let mut constraints = Vec::new();
		loop {
			let c_start = self.current_token_points().0;
			let c_trait_name = self.expect_identifier()?;
			let c_param = self.expect_identifier()?;
			constraints.push(InstanceConstraintNode {
				range: Range::between(c_start, c_param.range.end),
				trait_name: c_trait_name,
				param: c_param,
			});
			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
				}
				_ => break,
			}
		}
		expect_token_and_advance!(self, Token::RightParen);
		Some(constraints)
	}

	// Instance body: `implement TRAIT TYPE [where ...] { defs }`. The
	// `implement` keyword has already been consumed by the caller (which
	// captured its start point).
	fn parse_instance_after_implement(&mut self, start: Point) -> Option<DefinitionNode> {
		let trait_name = self.expect_identifier()?;

		// Head is a type expression so phase 3 can accept `(option a)` without
		// changing this slot's shape. Phase 1 only takes simple type names.
		// Use the non-greedy variant so the `{` that starts the body isn't
		// mistaken for a record-type generic arg.
		let head = self.parse_type_expression()?;

		// Optional `where (constraint, constraint, ...)` clause.
		let where_clause = self.parse_where_clause()?;

		let (_, _) = expect_token_and_advance!(self, Token::LeftBrace);
		self.skip_line_breaks();

		let mut methods: Vec<DefinitionNode> = Vec::new();
		while matches!(self.current_token, Some(Token::KeywordDef(..))) {
			let def = self.parse_definition()?;
			methods.push(def);
			self.skip_line_breaks();
		}

		let (_, brace_end) = expect_token_and_advance!(self, Token::RightBrace);
		self.skip_line_breaks();

		let instance_range = Range::between(start, brace_end);
		// Synthetic "name" for the def node — never appears in user code, but
		// the surrounding DefinitionNode requires one. Include the head's
		// span so two instances of the same trait don't collide in the
		// analyzer's duplicate-name check.
		let synthesized_name = IdentifierNode {
			range: trait_name.range,
			name: format!(
				"{}@instance@{}:{}",
				trait_name.name, head.range.start.line, head.range.start.col
			),
		};

		Some(DefinitionNode {
			name: synthesized_name,
			range: instance_range,
			kind: DefinitionKind::Instance(InstanceNode {
				range: instance_range,
				trait_name,
				head,
				where_clause,
				methods,
				instance_slot_name: String::new(),
				canonical_method_order: Vec::new(),
			}),
			visibility: Visibility::Private,
			type_annotation: None,
			where_clause: Vec::new(),
			ty: Type::Unknown,
			dict_param_count: 0,
		})
	}

	// `built-in "tag"`. The string literal must be plain (no
	// interpolation, no escapes that change content). The analyzer
	// requires this to appear as the immediate RHS of a type-annotated
	// top-level def; legality at that level is checked there.
	fn parse_builtin(&mut self) -> Option<ExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordBuiltin);
		let string_expr = self.parse_string()?;
		let tag = match &string_expr.kind {
			ExprKind::Literal(literal) => match &literal.kind {
				LiteralKind::String(value) => value.clone(),
				_ => {
					return self.error(ParseError {
						range: string_expr.range,
						kind: ParseErrorKind::BuiltinExpectsPlainString,
					});
				}
			},
			_ => {
				return self.error(ParseError {
					range: string_expr.range,
					kind: ParseErrorKind::BuiltinExpectsPlainString,
				});
			}
		};
		Some(ExprNode {
			range: Range::between(start, string_expr.range.end),
			kind: ExprKind::Builtin(tag),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		})
	}

	fn parse_string(&mut self) -> Option<ExprNode> {
		// There's a bit of trickiness here around start/end offsets. The token start/end
		// refers to the "readable" portion of the token (i.e. not including any surrounding
		// quotes). To get the full span of a basic string literal, with quotes on both sides,
		// we'd just do (start - 1, end + 1). But string literals that appear in the middle of
		// interpolations don't have quotes on either side, so things work a little differently.

		let (start, end) = expect_token_and_advance!(self, Token::StringLiteral);
		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let value = read_string_with_escapes!(self, start_offset, end_offset);

		let end = if current_token_is!(self, Token::InterpolationStart) {
			end
		} else {
			Point::at(end.line, end.col + 1)
		};

		let literal = LiteralNode {
			range: Range::between(start, end), // TODO off-by-one?
			kind: LiteralKind::String(value),
		};

		let expr_node = ExprNode {
			range: literal.range,
			kind: ExprKind::Literal(literal),
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
		};

		// If we have an interpolation-start after this, we need to collect all
		// the parts and return an interpolation expression, not just a literal
		// expression for the part we already found.
		if current_token_is!(self, Token::InterpolationStart) {
			let mut parts = vec![expr_node];
			let mut interpolation_end = end;

			while current_token_is!(self, Token::InterpolationStart) {
				self.advance();

				match self.parse_expression() {
					Some(node) => parts.push(node),
					_ => break,
				}

				expect_token_and_advance!(self, Token::InterpolationEnd);

				let (start, end) = expect_token_and_advance!(self, Token::StringLiteral);

				let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

				let part_range = Range::between(start, end);

				interpolation_end = end;

				let value = read_string_with_escapes!(self, start_offset, end_offset);

				parts.push(ExprNode {
					range: part_range,
					kind: ExprKind::Literal(LiteralNode {
						range: part_range,
						kind: LiteralKind::String(value),
					}),
					ty: Type::Unknown,
					trait_dispatch: None,
					dispatch_sink: None,
				});
			}

			return Some(ExprNode {
				range: Range::between(start, interpolation_end),
				kind: ExprKind::Interpolation(parts),
				ty: Type::Unknown,
				trait_dispatch: None,
				dispatch_sink: None,
			});
		}

		Some(expr_node)
	}

	// Bytes literals: `'...'`. No interpolation, no nesting. Escapes
	// recognized: \\, \', \0, \t, \r, \n, \xNN. Non-ASCII source bytes
	// pass through unchanged (the source file's UTF-8 encoding is what
	// lands in the literal).
	fn parse_bytes(&mut self) -> Option<ExprNode> {
		let (start, end) = expect_token_and_advance!(self, Token::BytesLiteral);
		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let mut bytes: Vec<u8> = Vec::with_capacity(end_offset - start_offset);
		let mut i = start_offset;
		while i < end_offset {
			let b = self.source[i];
			if b != b'\\' {
				bytes.push(b);
				i += 1;
				continue;
			}
			// `\` always pairs with the next byte; the tokenizer would have
			// terminated the literal otherwise. Read the escape kind.
			if i + 1 >= end_offset {
				// Trailing backslash. Shouldn't reach here given the
				// tokenizer's escape handling, but be safe.
				return self.error(ParseError {
					range: self.span_to_single_line_range(i, end_offset),
					kind: ParseErrorKind::InvalidBytesEscape,
				});
			}
			let esc = self.source[i + 1];
			match esc {
				b'\\' => bytes.push(b'\\'),
				b'\'' => bytes.push(b'\''),
				b'0' => bytes.push(0),
				b't' => bytes.push(b'\t'),
				b'r' => bytes.push(b'\r'),
				b'n' => bytes.push(b'\n'),
				b'x' => {
					if i + 3 >= end_offset {
						return self.error(ParseError {
							range: self.span_to_single_line_range(i, end_offset),
							kind: ParseErrorKind::InvalidHexEscape,
						});
					}
					let hi = hex_digit(self.source[i + 2]);
					let lo = hex_digit(self.source[i + 3]);
					match (hi, lo) {
						(Some(h), Some(l)) => bytes.push((h << 4) | l),
						_ => {
							return self.error(ParseError {
								range: self.span_to_single_line_range(i, i + 4),
								kind: ParseErrorKind::InvalidHexEscape,
							});
						}
					}
					i += 4;
					continue;
				}
				_ => {
					return self.error(ParseError {
						range: self.span_to_single_line_range(i, i + 2),
						kind: ParseErrorKind::InvalidBytesEscape,
					});
				}
			}
			i += 2;
		}

		// The token spans the inner content only (between the quotes).
		// Extend by one column on each side so the range covers both
		// quotes, matching how string literal ranges work.
		let range_start = Point::at(start.line, start.col.saturating_sub(1));
		let range_end = Point::at(end.line, end.col + 1);
		let range = Range::between(range_start, range_end);

		Some(ExprNode {
			range,
			ty: Type::Unknown,
			trait_dispatch: None,
			dispatch_sink: None,
			kind: ExprKind::Literal(LiteralNode {
				range,
				kind: LiteralKind::Bytes(bytes),
			}),
		})
	}

	fn parse_type_identifier(&mut self) -> Option<TypeIdentifierNode> {
		let (start, mut end) = expect_token_and_advance!(self, Token::Identifier);

		let (start_offset, end_offset) = (self.point_to_offset(start), self.point_to_offset(end));

		let first_name = read_string!(self, start_offset, end_offset);

		// Optional `module.TypeName` prefix. Dot in type position has no
		// other meaning, so when we see one we eagerly consume and expect an
		// identifier on the other side.
		let (module, name) = if matches!(self.current_token, Some(Token::Dot(..))) {
			let module_ident = IdentifierNode {
				range: Range::between(start, end),
				name: first_name,
			};
			self.advance();
			let (type_start, type_end) = expect_token_and_advance!(self, Token::Identifier);
			let (so, eo) = (
				self.point_to_offset(type_start),
				self.point_to_offset(type_end),
			);
			let type_name = read_string!(self, so, eo);
			end = type_end;
			(Some(module_ident), type_name)
		} else {
			(None, first_name)
		};

		// Generics are space-separated in single-type contexts (alias
		// bodies, function return types, record field types, tuple
		// elements). They're applied in `parse_type_expression_with_generics`,
		// not here — `parse_type_identifier` always produces a bare ident
		// with no generics. Multi-arg contexts (function params, variant
		// params) call `parse_type_expression` directly, so adjacent type
		// atoms read as separate params; parens are required around a
		// generic-applied type there.
		Some(TypeIdentifierNode {
			range: Range::between(start, end),
			module,
			name,
			generics: Vec::new(),
		})
	}

	// Parse a single-type context (alias body, function return, record field,
	// tuple element). Greedily consumes adjacent type atoms as generic args on
	// the head identifier — `result int string` parses as `result<int, string>`.
	// Each generic arg is itself a non-greedy atom; to nest, use parens
	// (`list (option int)` for `list<option<int>>`).
	fn parse_type_expression_with_generics(&mut self) -> Option<TypeExprNode> {
		let head = self.parse_type_expression()?;
		let head_start = head.range.start;

		// Only TypeIdentifier-shaped heads can take generics.
		let mut ident = match head.kind {
			TypeExprKind::Single(ident) => ident,
			_ => return Some(head),
		};

		let mut end = ident.range.end;
		while matches!(
			self.current_token,
			Some(
				Token::Identifier(..) | Token::LeftParen(..) | Token::LeftBrace(..) | Token::KeywordFun(..)
			)
		) {
			let arg = self.parse_type_expression()?;
			end = arg.range.end;
			ident.generics.push(arg);
		}

		ident.range = Range::between(ident.range.start, end);
		Some(TypeExprNode {
			range: Range::between(head_start, end),
			kind: TypeExprKind::Single(ident),
		})
	}

	fn parse_type_expression(&mut self) -> Option<TypeExprNode> {
		match self.current_token {
			Some(Token::Identifier(..)) => self.parse_type_identifier().map(|type_id| TypeExprNode {
				range: type_id.range,
				kind: TypeExprKind::Single(type_id),
			}),
			Some(Token::LeftParen(..)) => self.parse_type_parenthetical(),
			Some(Token::LeftBrace(..)) => self.parse_type_record(),
			Some(Token::KeywordFun(..)) => self.parse_type_fun(),
			_ => None,
		}
	}

	fn parse_type_fun(&mut self) -> Option<TypeExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordFun);

		self.skip_line_breaks();

		let mut param_types = Vec::new();

		while let Some(type_expr) = self.parse_type_expression() {
			param_types.push(type_expr);
		}

		let (_, end) = expect_token_and_advance!(self, Token::Arrow);

		let return_type = match self.parse_type_expression_with_generics() {
			Some(type_expr) => Box::new(type_expr),
			_ => {
				return self.error(ParseError {
					range: Range::between(start, end),
					kind: ParseErrorKind::MissingReturnType,
				})
			}
		};

		let range_end = return_type.range.end;

		Some(TypeExprNode {
			range: Range::between(start, range_end),
			kind: TypeExprKind::Func(param_types, return_type),
		})
	}

	fn parse_type_record(&mut self) -> Option<TypeExprNode> {
		let (record_start, _) = expect_token_and_advance!(self, Token::LeftBrace);

		self.skip_line_breaks();

		let mut entries = Vec::new();

		while let Some(field_name) = self.parse_identifier() {
			expect_token_and_advance!(self, Token::DoubleColon);

			let field_type = self.parse_type_expression_with_generics()?;

			entries.push((field_name, field_type));

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
				}
				Some(Token::LineBreak(..)) => {
					self.skip_line_breaks();
				}
				_ => break,
			}
		}

		self.skip_line_breaks();

		let (_, record_end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(TypeExprNode {
			range: Range::between(record_start, record_end),
			kind: TypeExprKind::Record(entries),
		})
	}

	fn parse_type_parenthetical(&mut self) -> Option<TypeExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::LeftParen);

		self.skip_line_breaks();

		let mut entries = Vec::new();

		while let Some(type_node) = self.parse_type_expression_with_generics() {
			entries.push(type_node);

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
				}
				Some(Token::LineBreak(..)) => {
					self.skip_line_breaks();
				}
				_ => break,
			}
		}

		self.skip_line_breaks();

		let (_, end) = expect_token_and_advance!(self, Token::RightParen);

		if entries.is_empty() {
			return Some(TypeExprNode {
				range: Range::between(start, end),
				kind: TypeExprKind::EmptyTuple,
			});
		}

		if entries.len() == 1 {
			if let Some(first_entry) = entries.pop() {
				return Some(TypeExprNode {
					range: Range::between(start, end),
					kind: TypeExprKind::Grouping(Box::new(first_entry)),
				});
			}
		}

		Some(TypeExprNode {
			range: Range::between(start, end),
			kind: TypeExprKind::Tuple(entries),
		})
	}

	fn parse_enum(&mut self) -> Option<EnumNode> {
		// Optional space-separated type params between `enum` and `{`:
		// `def opt enum a { some a; none }`. Bare identifiers only — the
		// `{` ends the param list.
		let mut params = Vec::new();
		while current_token_is!(self, Token::Identifier) {
			params.push(self.parse_identifier()?);
		}

		let (brace_start, _) = expect_token_and_advance!(self, Token::LeftBrace);

		self.skip_line_breaks();

		let mut variants = Vec::new();

		while let Some(variant) = self.parse_enum_variant() {
			variants.push(variant);

			self.skip_line_breaks();
		}

		let (_, brace_end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(EnumNode {
			range: Range::between(brace_start, brace_end),
			params,
			variants,
		})
	}

	fn parse_enum_variant_name(&mut self) -> Option<IdentifierNode> {
		let (start, end) = match self.current_token {
			Some(Token::Identifier(start, end))
			| Some(Token::BoolTrue(start, end))
			| Some(Token::BoolFalse(start, end)) => (start, end),
			_ => return None,
		};

		self.advance();
		let name = read_string!(self, start, end);

		Some(IdentifierNode {
			range: self.span_to_single_line_range(start, end),
			name,
		})
	}

	fn parse_enum_variant(&mut self) -> Option<EnumVariantNode> {
		let name = self.parse_enum_variant_name()?;

		if current_token_is!(self, Token::LineBreak) {
			self.skip_line_breaks();

			return Some(EnumVariantNode {
				range: Range::between(name.range.start, name.range.end),
				name,
				params: None,
			});
		}

		let mut params = Vec::new();

		while let Some(type_node) = self.parse_type_expression() {
			params.push(type_node);

			match self.current_token {
				Some(Token::Comma(..)) => self.advance(),
				Some(Token::LineBreak(..))
				| Some(Token::LineBreakWithIndentDecrease(..))
				| Some(Token::LineBreakWithIndentIncrease(..))
				| Some(Token::RightBrace(..))
				| None => break,
				_ => {}
			}
		}

		let end = params.last().map(|p| p.range.end).unwrap_or(name.range.end);

		Some(EnumVariantNode {
			range: Range::between(name.range.start, end),
			name,
			params: if params.is_empty() {
				None
			} else {
				Some(params)
			},
		})
	}
}
