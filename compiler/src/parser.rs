use crate::ast::*;
use crate::errors::*;
use crate::location::Point;
use crate::location::Range;
use crate::tokenizer::Tokenizer;
use crate::tokens::Token;
use crate::types::*;
use std::collections::{HashMap, VecDeque};

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
		// looking for them.
		loop {
			self.skip_line_breaks();
			match self.current_token {
				Some(Token::KeywordUse(..)) => match self.parse_use() {
					Some(u) => uses.push(u),
					None => break,
				},
				_ => break,
			}
		}

		loop {
			self.skip_line_breaks();

			match self.parse_definition() {
				Some(definition) => body.push(definition),
				_ => break,
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
		path.push(self.parse_identifier()?);

		while matches!(self.current_token, Some(Token::Dot(..))) {
			self.advance();
			path.push(self.parse_identifier()?);
		}

		let alias = if matches!(self.current_token, Some(Token::KeywordAs(..))) {
			self.advance();
			Some(self.parse_identifier()?)
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
		self.current_token = self.lookahead.pop_front().or_else(|| self.tokenizer.next());
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
			match self.tokenizer.next() {
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

	fn parse_body_expressions(&mut self) -> Option<Vec<ExprNode>> {
		let mut body = Vec::new();

		loop {
			self.skip_line_breaks();

			if let Some(node) = self.parse_expression() {
				body.push(node);
			} else {
				break;
			}
		}

		Some(body)
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
			Some(Token::ForwardSlash(..)) => self.parse_regular_expression(),
			Some(Token::StringLiteral(..)) => self.parse_string(),
			Some(Token::BoolTrue(..)) => self.parse_bool(),
			Some(Token::BoolFalse(..)) => self.parse_bool(),
			Some(Token::KeywordWhen(..)) => self.parse_when_expression().map(|when_node| ExprNode {
				range: when_node.range,
				kind: ExprKind::When(when_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordIf(..)) => self.parse_if_expression().map(|if_node| ExprNode {
				range: if_node.range,
				kind: ExprKind::If(if_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordWhile(..)) => self.parse_while_expression().map(|while_node| ExprNode {
				range: while_node.range,
				kind: ExprKind::While(while_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordLet(..)) => self.parse_let_expression().map(|let_node| ExprNode {
				range: let_node.range,
				kind: ExprKind::Let(let_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordFun(..)) => self.parse_fun().map(|fun_node| ExprNode {
				range: fun_node.range,
				kind: ExprKind::Fun(fun_node),
				ty: Type::Unknown,
			}),
			Some(Token::Identifier(..)) => self.parse_identifier().map(|ident| ExprNode {
				range: ident.range,
				kind: ExprKind::Identifier(ident),
				ty: Type::Unknown,
			}),
			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(Token::HexDigits(..)) => self.parse_hex_number().map(|literal| ExprNode {
				range: literal.range,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(t @ Token::Minus(start, ..) | t @ Token::Bang(start, ..)) => {
				// these are prefix unary operators!
				let operator = Operator::from_token(t).unwrap();
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

					while let Some(arg_expr) = self.parse_expression_with_binding_power(right_bp) {
						args.push(arg_expr);
					}

					// We entered FunctionCall because `can_start_expression`
					// said the next token could begin one, but couldn't actually
					// parse an arg — give up and let the outer parser report a
					// useful error on whatever's there.
					if args.is_empty() {
						break;
					}

					let range = Range::between(
						lhs_expr.range.start,
						args.last().unwrap().range.end,
					);

					lhs_expr = ExprNode {
						range,
						kind: ExprKind::Call(CallNode {
							range,
							callee: Box::new(lhs_expr),
							args,
						}),
						ty: Type::Unknown,
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
				kind: ExprKind::ElementAccess {
					receiver: lhs_expr.into(),
					index,
				},
			}),

			ExprKind::Identifier(ident) => Some(ExprNode {
				range: Range::between(lhs_expr.range.start, rhs_expr.range.end),
				ty: Type::Unknown,
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

	fn parse_if_expression(&mut self) -> Option<IfNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordIf);

		let condition = self.parse_expression()?;

		expect_token_and_advance!(self, Token::KeywordIs);

		let pattern = self.parse_pattern()?;

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(IfNode {
			range: Range::between(start, end),
			subject: Box::new(condition),
			pattern,
			body,
		})
	}

	fn parse_when_expression(&mut self) -> Option<WhenNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordWhen);

		let subject = self.parse_expression()?;

		self.skip_line_breaks();

		let mut cases = Vec::new();

		while current_token_is!(self, Token::KeywordIs) {
			let case_start = self.offset_to_point(self.current_token_span().0);

			self.advance();

			let case_pattern = self.parse_pattern()?;

			expect_token_and_advance!(self, Token::LeftBrace);

			let case_body = self.parse_body_expressions()?;

			self.skip_line_breaks();

			let (_, case_end) = expect_token_and_advance!(self, Token::RightBrace);

			self.skip_line_breaks();

			cases.push(CaseNode {
				range: Range::between(case_start, case_end),
				pattern: case_pattern,
				body: case_body,
			})
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

			Some(Token::LeftBrace(start_offset, _)) => {
				let start = self.offset_to_point(start_offset);

				self.advance();

				let mut entries = Vec::new();

				while let Some(field_name) = self.parse_identifier() {
					expect_token_and_advance!(self, Token::Colon);

					let field_pattern = self.parse_pattern()?;

					entries.push((field_name, field_pattern));

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

				let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

				Some(PatternNode {
					range: Range::between(start, end),
					kind: PatternKind::Record(entries),
				})
			}

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

			_ => None,
		}
	}

	fn parse_let_expression(&mut self) -> Option<LetNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordLet);

		let name = match self.parse_identifier() {
			Some(node) => node,
			_ => todo!(),
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
			name,
			value: Box::new(value),
		})
	}

	fn parse_list(&mut self) -> Option<ExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::LeftBracket);

		self.skip_line_breaks();

		let mut elements = Vec::new();

		while let Some(expr) = self.parse_expression() {
			elements.push(expr);

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
			expect_token_and_advance!(self, Token::Colon);

			let field_value = self.parse_expression()?;

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
			});
		}

		if entries.len() == 1 {
			// If only one expression was found, it's a grouping
			if let Some(first_expr) = entries.pop() {
				return Some(ExprNode {
					range: Range::between(paren_start, paren_end),
					kind: ExprKind::Grouping(Box::new(first_expr)),
					ty: Type::Unknown,
				});
			}
		}

		// Otherwise, it's a tuple with multiple entries:
		Some(ExprNode {
			range: Range::between(paren_start, paren_end),
			kind: ExprKind::Tuple(entries),
			ty: Type::Unknown,
		})
	}

	fn parse_regular_expression(&mut self) -> Option<ExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::ForwardSlash);

		self.skip_line_breaks();

		let maybe_reg_expr_node = self.parse_regular_expression_body();

		self.skip_line_breaks();

		let (_, end) = expect_token_and_advance!(self, Token::ForwardSlash);

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

			let part = match self.current_token {
				Some(Token::Identifier(start_offset, end_offset)) => {
					self.advance();

					let name = read_string!(self, start_offset, end_offset);

					RegexNode {
						range: self.span_to_single_line_range(start_offset, end_offset),
						kind: RegexKind::CharacterClass(name),
					}
				}

				Some(Token::StringLiteral(start_offset, end_offset)) => {
					self.advance();

					let value = read_string_with_escapes!(self, start_offset, end_offset);

					RegexNode {
						range: self.span_to_single_line_range(start_offset, end_offset),
						kind: RegexKind::Literal(value),
					}
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

					RegexNode {
						range: Range::between(start, end),
						kind: RegexKind::Grouping(Box::new(expr)),
					}
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

					RegexNode {
						range: Range::between(start, end),
						kind: RegexKind::NamedCapture(name, Box::new(expr)),
					}
				}

				_ => break,
			};

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
		let start = match self.current_token {
			Some(Token::KeywordDef(start_offset, _)) => {
				self.advance();
				self.offset_to_point(start_offset)
			}
			_ => return None,
		};

		let name = self.parse_identifier()?;

		match self.current_token {
			Some(Token::KeywordAlias(..)) => {
				self.advance();

				let type_expr = self.parse_type_expression()?;

				self.skip_line_breaks();

				Some(DefinitionNode {
					name,
					range: Range::between(start, type_expr.range.end),
					kind: DefinitionKind::Alias(type_expr),
					ty: Type::Unknown,
				})
			}

			Some(Token::KeywordEnum(..)) => {
				self.advance();

				let enum_node = self.parse_enum()?;

				self.skip_line_breaks();

				Some(DefinitionNode {
					name,
					range: Range::between(start, enum_node.range.end),
					kind: DefinitionKind::Enum(enum_node),
					ty: Type::Unknown,
				})
			}

			Some(token) if token.can_start_expression() => {
				let value = self.parse_expression()?;

				self.skip_line_breaks();

				Some(DefinitionNode {
					name,
					range: Range::between(start, value.range.end),
					kind: DefinitionKind::Expr(value),
					ty: Type::Unknown,
				})
			}

			_ => self.error(ParseError {
				range: Range::between(start, self.current_token_points().1),
				kind: ParseErrorKind::InvalidDefBody,
			}),
		}
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
				});
			}

			return Some(ExprNode {
				range: Range::between(start, interpolation_end),
				kind: ExprKind::Interpolation(parts),
				ty: Type::Unknown,
			});
		}

		Some(expr_node)
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

		let mut generics = Vec::new();

		if current_token_is!(self, Token::LeftAngle) {
			self.advance();

			while let Some(type_node) = self.parse_type_expression() {
				generics.push(type_node);

				match self.current_token {
					Some(Token::Comma(..)) => self.advance(),
					_ => break,
				}
			}

			let (_, angle_end) = expect_token_and_advance!(self, Token::RightAngle);

			end = angle_end;
		}

		Some(TypeIdentifierNode {
			range: Range::between(start, end),
			module,
			name,
			generics,
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

		let return_type = match self.parse_type_expression() {
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
			expect_token_and_advance!(self, Token::Colon);

			let field_type = self.parse_type_expression()?;

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

		while let Some(type_node) = self.parse_type_expression() {
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
