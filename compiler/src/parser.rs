use crate::ast::*;
use crate::errors::*;
use crate::tokenizer::Tokenizer;
use crate::tokens::Token;
use crate::typing::*;
use std::collections::HashMap;

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
				(start, end)
			}
			Some(tok) => {
				return $self.error(ParseError {
					span: tok.get_span(),
					kind: ParseErrorKind::UnexpectedToken {
						actual: tok,
						expected: $tokType(0, 0),
					},
				});
			}
			None => {
				return $self.error(ParseError {
					span: ($self.source.len(), $self.source.len()),
					kind: ParseErrorKind::UnexpectedEOF {
						expected: $tokType(0, 0),
					},
				});
			}
		}
	};
}

macro_rules! read_string {
	($self:ident, $start:expr, $end:expr) => {
		String::from_utf8($self.source[$start..$end].to_vec()).expect("not utf-8")
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
	line_breaks: Vec<Span>,
}

impl<'a> Parser<'a> {
	pub fn new(source: &'a Vec<u8>, tokenizer: Tokenizer<'a>) -> Parser<'a> {
		return Parser {
			source,
			tokenizer,
			errors: Vec::new(),
			current_token: None,
			prev_token: None,
			line_breaks: Vec::new(),
		};
	}

	pub fn parse_module(&mut self) -> (ModuleNode, HashMap<usize, String>, Vec<ParseError>) {
		let mut body = Vec::new();

		// Read the first token
		self.advance();

		loop {
			self.skip_line_breaks();

			match self.parse_definition() {
				Some(definition) => body.push(definition),
				_ => break,
			}
		}

		if let Some(extra_token) = self.current_token {
			self.error::<ModuleNode>(ParseError {
				span: self.current_token_span(),
				kind: ParseErrorKind::UnexpectedTokenExpectedEOF {
					actual: extra_token,
				},
			});
		}

		let start = body.first().map_or(0, |node| node.span.0);
		let end = body.last().map_or(0, |node| node.span.1);

		let module_node = ModuleNode {
			span: (start, end),
			body,
		};

		(
			module_node,
			self.tokenizer.comments.clone(),
			self.errors.clone(),
		)
	}

	fn advance(&mut self) {
		self.prev_token = self.current_token;
		self.current_token = self.tokenizer.next();
	}

	fn skip_line_breaks(&mut self) {
		match &self.current_token {
			Some(
				Token::LineBreak(..)
				| Token::LineBreakWithIndentIncrease(..)
				| Token::LineBreakWithIndentDecrease(..),
			) => {
				self.line_breaks.push(self.current_token_span());
				self.advance();
			}

			_ => {}
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
			}

			if current_token_is!(self, Token::LineBreak) {
				self.advance();
				self.skip_line_breaks();
			} else {
				break;
			}
		}

		Some(body)
	}

	fn parse_lambda(&mut self) -> Option<LambdaNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordFun);

		let mut params = Vec::new();

		// TODO: allow patterns here, not just identifiers
		while current_token_is!(self, Token::Identifier) {
			let ident = self.parse_identifier()?;

			params.push(LambdaParamNode {
				ident,
				ty: Type::Unknown,
			});
		}

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		self.skip_line_breaks();

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(LambdaNode {
			span: (start, end),
			params,
			body,
		})
	}

	fn parse_decimal_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::DecimalDigits);

		if current_token_is!(self, Token::Dot) {
			self.advance();

			let (_, end) = expect_token_and_advance!(self, Token::DecimalDigits);

			let str_value = read_string!(self, start, end);
			let float_value = str_value.parse::<f64>().unwrap();

			return Some(LiteralNode {
				kind: LiteralKind::FloatDecimal(float_value),
				span: (start, end),
			});
		}

		let value = self.parse_numeric_literal(start, end, 10);

		Some(LiteralNode {
			kind: LiteralKind::IntDecimal(value),
			span: (start, end),
		})
	}

	fn parse_binary_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::BinaryDigits);

		let value = self.parse_numeric_literal(start + 2, end, 2);

		Some(LiteralNode {
			kind: LiteralKind::IntBinary(value),
			span: (start, end),
		})
	}

	fn parse_octal_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::OctalDigits);

		let value = self.parse_numeric_literal(start + 2, end, 8);

		Some(LiteralNode {
			kind: LiteralKind::IntOctal(value),
			span: (start, end),
		})
	}

	fn parse_hex_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_advance!(self, Token::HexDigits);

		let value = self.parse_numeric_literal(start + 2, end, 16);

		Some(LiteralNode {
			kind: LiteralKind::IntHex(value),
			span: (start, end),
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
			Some(Token::KeywordWhen(..)) => self.parse_when_expression().map(|when_node| ExprNode {
				span: when_node.span,
				kind: ExprKind::When(when_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordIf(..)) => self.parse_if_expression().map(|when_node| ExprNode {
				span: when_node.span,
				kind: ExprKind::If(when_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordWhile(..)) => self.parse_while_expression().map(|while_node| ExprNode {
				span: while_node.span,
				kind: ExprKind::While(while_node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordLet(..)) => self.parse_let_expression().map(|node| ExprNode {
				span: node.span,
				kind: ExprKind::Let(node),
				ty: Type::Unknown,
			}),
			Some(Token::KeywordFun(..)) => self.parse_lambda().map(|lambda| ExprNode {
				span: lambda.span,
				kind: ExprKind::Lambda(lambda),
				ty: Type::Unknown,
			}),
			Some(Token::Identifier(..)) => self.parse_identifier().map(|ident| ExprNode {
				span: ident.span,
				kind: ExprKind::Identifier(ident),
				ty: Type::Unknown,
			}),
			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|literal| ExprNode {
				span: literal.span,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|literal| ExprNode {
				span: literal.span,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|literal| ExprNode {
				span: literal.span,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(Token::HexDigits(..)) => self.parse_hex_number().map(|literal| ExprNode {
				span: literal.span,
				kind: ExprKind::Literal(literal),
				ty: Type::Unknown,
			}),
			Some(t @ Token::Minus(start, ..) | t @ Token::Bang(start, ..)) => {
				// these are prefix unary operators!
				let operator = Operator::from_token(t).unwrap();
				self.advance();

				// make sure to parse the expression following the operator with
				// the correct binding power:
				let (_, right_bp) = operator.prefix_binding_power();
				let rhs_expr = self.parse_expression_with_binding_power(right_bp)?;

				Some(ExprNode {
					span: (start, start),
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
			if current_token_is!(self, Token::LineBreak)
				|| current_token_is!(self, Token::LineBreakWithIndentDecrease)
			{
				break;
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

					let start = lhs_expr.span.0;
					let end = args.last().expect("at least one arg").span.1;

					lhs_expr = ExprNode {
						span: (start, end),
						kind: ExprKind::Call(CallNode {
							span: (start, end),
							callee: Box::new(lhs_expr),
							args,
						}),
						ty: Type::Unknown,
					};
				} else {
					let op_pos = self.current_token_span();

					// advance past the operator token
					self.advance();

					let rhs_expr = self.parse_expression_with_binding_power(right_bp)?;

					if let Operator::IndexAccess = operator {
						// special case: the [ operator needs a closing ]
						expect_token_and_advance!(self, Token::RightBracket);
					}

					lhs_expr = ExprNode {
						span: (lhs_expr.span.0, rhs_expr.span.1),
						kind: ExprKind::BinaryOperation {
							op: OperatorNode {
								span: op_pos,
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
			span: (start, end),
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
			span: (start, end),
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
			let case_start = self.current_token_span().0;

			self.advance();

			let case_pattern = self.parse_pattern()?;

			expect_token_and_advance!(self, Token::LeftBrace);

			let case_body = self.parse_body_expressions()?;

			let (_, case_end) = expect_token_and_advance!(self, Token::RightBrace);

			self.skip_line_breaks();

			cases.push(CaseNode {
				span: (case_start, case_end),
				pattern: case_pattern,
				body: case_body,
			})
		}

		// TODO: error if 0 cases
		let end = cases.last().expect("> 0 cases").span.1;

		Some(WhenNode {
			span: (start, end),
			subject: Box::new(subject),
			cases,
		})
	}

	fn parse_while_expression(&mut self) -> Option<WhileNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordWhile);

		let condition = self.parse_expression()?;

		expect_token_and_advance!(self, Token::KeywordIs);

		let pattern = self.parse_pattern()?;

		expect_token_and_advance!(self, Token::LeftBrace);

		let body = self.parse_body_expressions()?;

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(WhileNode {
			span: (start, end),
			condition: Box::new(condition),
			pattern,
			body,
		})
	}

	fn parse_pattern(&mut self) -> Option<PatternNode> {
		match self.current_token {
			Some(Token::Identifier(..)) => {
				let id_node = self.parse_identifier().unwrap();

				// TODO: handle constructors with multiple args
				if let Some(arg_pattern) = self.parse_pattern() {
					return Some(PatternNode {
						span: (id_node.span.0, arg_pattern.span.1),
						kind: PatternKind::Constructor(id_node, Box::new(arg_pattern)),
					});
				}

				Some(PatternNode {
					span: id_node.span,
					kind: PatternKind::Identifier(id_node),
				})
			}

			Some(Token::LeftParen(start, _)) => {
				self.advance();

				let mut entries = Vec::new();

				while let Some(pattern) = self.parse_pattern() {
					entries.push(pattern);

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

				Some(PatternNode {
					span: (start, end),
					kind: PatternKind::Tuple(entries),
				})
			}

			Some(Token::LeftBrace(start, _)) => {
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
					span: (start, end),
					kind: PatternKind::Record(entries),
				})
			}

			Some(Token::Underscore(start, end)) => {
				self.advance();

				Some(PatternNode {
					span: (start, end),
					kind: PatternKind::Underscore,
				})
			}

			Some(Token::StringLiteral(..)) => self.parse_string().map(|expr_node| match expr_node.kind {
				ExprKind::Literal(literal) => PatternNode {
					span: literal.span,
					kind: PatternKind::Literal(literal),
				},
				ExprKind::Interpolation(parts) => PatternNode {
					span: expr_node.span,
					kind: PatternKind::Interpolation(parts),
				},
				_ => unreachable!(),
			}),

			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| PatternNode {
				span: lit_node.span,
				kind: PatternKind::Literal(lit_node),
			}),

			// TODO: other kinds of digits here
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
			Some(node) => (node.span.1, node),
			_ => {
				return self.error(ParseError {
					span: self.current_token_span(),
					kind: ParseErrorKind::MissingRightHandSideOfAssignment,
				})
			}
		};

		Some(LetNode {
			span: (start, end),
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
			span: (start, end),
			kind: ExprKind::List(elements),
			ty: Type::Unknown,
		})
	}

	fn parse_numeric_literal(&mut self, start: usize, end: usize, radix: usize) -> usize {
		let mut result: usize = 0;
		let mut i: usize = 1;

		for byte in self.source[start..end].iter().rev() {
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
					span: (start, end),
					kind: ParseErrorKind::OverflowingIntegerLiteral,
				});
				return 0;
			}

			if let Some(next_i) = i.checked_mul(radix) {
				i = next_i;
			} else {
				self.error::<LiteralNode>(ParseError {
					span: (start, end),
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
			span: (record_start, record_end),
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
				span: (paren_start, paren_end),
				kind: ExprKind::EmptyTuple,
				ty: Type::Unknown,
			});
		}

		if entries.len() == 1 {
			// If only one expression was found, it's a grouping
			if let Some(first_expr) = entries.pop() {
				return Some(ExprNode {
					span: (paren_start, paren_end),
					kind: ExprKind::Grouping(Box::new(first_expr)),
					ty: Type::Unknown,
				});
			}
		}

		// Otherwise, it's a tuple with multiple entries:
		Some(ExprNode {
			span: (paren_start, paren_end),
			kind: ExprKind::Tuple(entries),
			ty: Type::Unknown,
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
					span: (start, end),
					kind: ParseErrorKind::EmptyRegularExpression,
				})
			}
		};

		Some(ExprNode {
			span: (start, end),
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

		let start = other_terms.first().unwrap().span.0;
		let end = other_terms.last().unwrap().span.1;

		Some(RegexNode {
			span: (start, end),
			kind: RegexKind::Alternation(other_terms),
		})
	}

	fn parse_regular_expression_term(&mut self) -> Option<RegexNode> {
		let mut first_part = None;
		let mut other_parts = Vec::new();

		loop {
			self.skip_line_breaks();

			let part = match self.current_token {
				Some(Token::Identifier(start, end)) => {
					self.advance();

					let name = read_string!(self, start, end);

					RegexNode {
						span: (start, end),
						kind: RegexKind::CharacterClass(name),
					}
				}

				Some(Token::StringLiteral(start, end)) => {
					self.advance();

					let value = read_string_with_escapes!(self, start, end);

					RegexNode {
						span: (start, end),
						kind: RegexKind::Literal(value),
					}
				}

				Some(Token::LeftParen(start, end)) => {
					self.advance();

					let expr = match self.parse_regular_expression_body() {
						Some(expr) => expr,
						None => {
							return self.error(ParseError {
								span: (start, end),
								kind: ParseErrorKind::EmptyRegularExpressionGroup,
							})
						}
					};

					expect_token_and_advance!(self, Token::RightParen);

					RegexNode {
						span: (start, end),
						kind: RegexKind::Grouping(Box::new(expr)),
					}
				}

				Some(Token::LeftAngle(start, end)) => {
					self.advance();

					let name_span = expect_token_and_advance!(self, Token::Identifier);
					let name = read_string!(self, name_span.0, name_span.1);

					expect_token_and_advance!(self, Token::Colon);

					let expr = match self.parse_regular_expression_body() {
						Some(expr) => expr,
						None => {
							return self.error(ParseError {
								span: (start, end),
								kind: ParseErrorKind::EmptyRegularExpressionGroup,
							})
						}
					};

					expect_token_and_advance!(self, Token::RightAngle);

					RegexNode {
						span: (start, end),
						kind: RegexKind::NamedCapture(name, Box::new(expr)),
					}
				}

				_ => break,
			};

			let modified_part = match self.current_token {
				Some(Token::Star(_, end)) => {
					self.advance();

					RegexNode {
						span: (part.span.0, end),
						kind: RegexKind::ZeroOrMore(Box::new(part)),
					}
				}

				Some(Token::Plus(_, end)) => {
					self.advance();

					RegexNode {
						span: (part.span.0, end),
						kind: RegexKind::OneOrMore(Box::new(part)),
					}
				}

				Some(Token::Question(_, end)) => {
					self.advance();

					RegexNode {
						span: (part.span.0, end),
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
							span: (part.span.0, end),
							kind: RegexKind::AtLeastCount(Box::new(part), min),
						},

						(None, Some(max), true) => RegexNode {
							span: (part.span.0, end),
							kind: RegexKind::AtMostCount(Box::new(part), max),
						},

						(Some(min), None, false) => RegexNode {
							span: (part.span.0, end),
							kind: RegexKind::ExactCount(Box::new(part), min),
						},

						(Some(min), Some(max), true) => {
							if min > max {
								self.error::<RegexNode>(ParseError {
									span: (part.span.0, end),
									kind: ParseErrorKind::InvalidRegularExpressionCountModifier,
								});
							}

							RegexNode {
								span: (part.span.0, end),
								kind: RegexKind::RangeCount(Box::new(part), min, max),
							}
						}

						_ => {
							return self.error(ParseError {
								span: (part.span.0, end),
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

		Some(RegexNode {
			span: (0, 0), // TODO!
			kind: RegexKind::Sequence(other_parts),
		})
	}

	fn parse_definition(&mut self) -> Option<DefinitionNode> {
		let start = match self.current_token {
			Some(Token::KeywordDef(start, _)) => {
				self.advance();
				start
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
					span: (start, type_expr.span.1),
					kind: DefinitionKind::Alias(type_expr),
					ty: Type::Unknown,
				})
			}

			_ => {
				let value = self.parse_expression()?;

				self.skip_line_breaks();

				Some(DefinitionNode {
					name,
					span: (start, value.span.1),
					kind: DefinitionKind::Expr(value),
					ty: Type::Unknown,
				})
			}
		}
	}

	fn parse_string(&mut self) -> Option<ExprNode> {
		let (start, end) = expect_token_and_advance!(self, Token::StringLiteral);

		let value = read_string_with_escapes!(self, start, end);

		let literal = LiteralNode {
			span: (start, end),
			kind: LiteralKind::Str(value),
		};

		let expr_node = ExprNode {
			span: (start, end),
			kind: ExprKind::Literal(literal),
			ty: Type::Unknown,
		};

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

				interpolation_end = end;

				let value = read_string_with_escapes!(self, start, end);

				parts.push(ExprNode {
					span: (start, end),
					kind: ExprKind::Literal(LiteralNode {
						span: (start, end),
						kind: LiteralKind::Str(value),
					}),
					ty: Type::Unknown,
				});
			}

			return Some(ExprNode {
				span: (start, interpolation_end),
				kind: ExprKind::Interpolation(parts),
				ty: Type::Unknown,
			});
		}

		Some(expr_node)
	}

	fn parse_type_identifier(&mut self) -> Option<TypeIdentifierNode> {
		let (start, mut end) = expect_token_and_advance!(self, Token::Identifier);

		let name = read_string!(self, start, end);

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
			span: (start, end),
			name,
			generics,
		})
	}

	fn parse_type_expression(&mut self) -> Option<TypeExprNode> {
		match self.current_token {
			Some(Token::Identifier(..)) => self.parse_type_identifier().map(|type_id| TypeExprNode {
				span: type_id.span,
				kind: TypeExprKind::Single(type_id),
			}),
			Some(Token::LeftParen(..)) => self.parse_type_parenthetical(),
			Some(Token::LeftBrace(..)) => self.parse_type_record(),
			Some(Token::KeywordFun(..)) => self.parse_type_lambda(),
			_ => None,
		}
	}

	fn parse_type_lambda(&mut self) -> Option<TypeExprNode> {
		let (start, _) = expect_token_and_advance!(self, Token::KeywordFun);

		self.skip_line_breaks();

		let mut param_types = Vec::new();

		while let Some(type_expr) = self.parse_type_expression() {
			param_types.push(type_expr);
		}

		expect_token_and_advance!(self, Token::Arrow);

		let return_type = match self.parse_type_expression() {
			Some(type_expr) => Box::new(type_expr),
			_ => {
				return self.error(ParseError {
					span: self.current_token_span(),
					kind: ParseErrorKind::MissingReturnType,
				})
			}
		};

		let (_, end) = expect_token_and_advance!(self, Token::RightBrace);

		Some(TypeExprNode {
			span: (start, end),
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
			span: (record_start, record_end),
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
				span: (start, end),
				kind: TypeExprKind::EmptyTuple,
			});
		}

		if entries.len() == 1 {
			if let Some(first_entry) = entries.pop() {
				return Some(TypeExprNode {
					span: (start, end),
					kind: TypeExprKind::Grouping(Box::new(first_entry)),
				});
			}
		}

		Some(TypeExprNode {
			span: (start, end),
			kind: TypeExprKind::Tuple(entries),
		})
	}
}
