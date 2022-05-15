use crate::ast::*;
use crate::errors::*;
use crate::expr_type::*;
use crate::tokenizer::Tokenizer;
use crate::tokens::Token;
use std::collections::HashMap;

macro_rules! current_token_is {
	($self:ident, $tokType:path) => {
		match $self.current_token {
			Some($tokType(..)) => true,
			_ => false,
		}
	};
}

macro_rules! next_token_is {
	($self:ident, $tokType:path) => {
		match $self.tokenizer.peek() {
			Some($tokType(..)) => true,
			_ => false,
		}
	};
}

macro_rules! expect_token_and_do {
	($self:ident, $tokType:path, $block:tt) => {
		match $self.current_token {
			Some($tokType(..)) => $block,
			Some(tok) => {
				return $self.error(ParseError {
					pos: tok.get_position(),
					kind: ParseErrorKind::UnexpectedToken {
						actual: tok,
						expected: $tokType(0, 0),
					},
				});
			}
			None => {
				return $self.error(ParseError {
					pos: ($self.source.len(), $self.source.len()),
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
	line_breaks: Vec<Position>,
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
				pos: self.current_token_position(),
				kind: ParseErrorKind::UnexpectedTokenExpectedEOF {
					actual: extra_token,
				},
			});
		}

		let start = body.first().map_or(0, |node| node.pos.0);
		let end = body.last().map_or(0, |node| node.pos.1);

		let module_node = ModuleNode {
			pos: (start, end),
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

	fn skip_line_breaks(&mut self) -> bool {
		let mut skipped_any = false;

		while current_token_is!(self, Token::LineBreak) {
			self.line_breaks.push(self.current_token_position());

			skipped_any = true;

			self.advance()
		}

		skipped_any
	}

	fn current_token_position(&self) -> (usize, usize) {
		match self.current_token {
			Some(token) => token.get_position(),
			_ => match self.prev_token {
				Some(token) => token.get_position(),
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

			if current_token_is!(self, Token::Semicolon) {
				self.advance();
				self.skip_line_breaks();
			} else {
				break;
			}
		}

		Some(body)
	}

	fn parse_lambda(&mut self) -> Option<LambdaNode> {
		let start = expect_token_and_do!(self, Token::KeywordFun, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		let mut params = Vec::new();

		// TODO: allow patterns here, not just identifiers
		while current_token_is!(self, Token::Identifier) {
			let param = self.parse_identifier()?;
			params.push(param);
		}

		expect_token_and_do!(self, Token::LeftBrace, {
			self.advance();
		});

		let body = self.parse_body_expressions()?;

		let end = expect_token_and_do!(self, Token::RightBrace, {
			let end = self.current_token_position().1;
			self.advance();
			end
		});

		Some(LambdaNode {
			pos: (start, end),
			params,
			body,
		})
	}

	fn parse_decimal_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_do!(self, Token::DecimalDigits, {
			let pos = self.current_token_position();
			self.advance();
			pos
		});

		if current_token_is!(self, Token::Dot) && next_token_is!(self, Token::DecimalDigits) {
			self.advance();

			expect_token_and_do!(self, Token::DecimalDigits, {
				let (_, end) = self.current_token_position();

				self.advance();

				let str_value = read_string!(self, start, end);
				let float_value = str_value.parse::<f64>().unwrap();

				return Some(LiteralNode {
					kind: LiteralKind::FloatDecimal(float_value),
					pos: (start, end),
				});
			});
		}

		let value = self.parse_numeric_literal(start, end, 10);

		Some(LiteralNode {
			kind: LiteralKind::IntDecimal(value),
			pos: (start, end),
		})
	}

	fn parse_binary_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_do!(self, Token::BinaryDigits, {
			let pos = self.current_token_position();
			self.advance();
			pos
		});

		let value = self.parse_numeric_literal(start + 2, end, 2);

		Some(LiteralNode {
			kind: LiteralKind::IntBinary(value),
			pos: (start, end),
		})
	}

	fn parse_octal_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_do!(self, Token::OctalDigits, {
			let pos = self.current_token_position();
			self.advance();
			pos
		});

		let value = self.parse_numeric_literal(start + 2, end, 8);

		Some(LiteralNode {
			kind: LiteralKind::IntOctal(value),
			pos: (start, end),
		})
	}

	fn parse_hex_number(&mut self) -> Option<LiteralNode> {
		let (start, end) = expect_token_and_do!(self, Token::HexDigits, {
			let pos = self.current_token_position();
			self.advance();
			pos
		});

		let value = self.parse_numeric_literal(start + 2, end, 16);

		Some(LiteralNode {
			kind: LiteralKind::IntHex(value),
			pos: (start, end),
		})
	}

	fn parse_expression(&mut self) -> Option<ExprNode> {
		self.parse_expression_with_binding_power(0)
	}

	fn parse_expression_with_binding_power(&mut self, min_bp: u8) -> Option<ExprNode> {
		let mut lhs_expr = match self.current_token {
			Some(Token::LeftParen(..)) => self.parse_parenthetical(),
			Some(Token::LeftBracket(..)) => self.parse_list(),
			// Some(Token::LeftBrace(..)) => self.parse_dict(),
			Some(Token::Backtick(..)) => self.parse_regular_expression(),
			Some(Token::StringLiteral(..)) => self.parse_string(),
			Some(Token::KeywordWhen(..)) => self.parse_when_expression().map(|when_node| ExprNode {
				pos: when_node.pos,
				kind: ExprKind::When(when_node),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::KeywordIf(..)) => self.parse_if_expression().map(|when_node| ExprNode {
				pos: when_node.pos,
				kind: ExprKind::If(when_node),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::KeywordWhile(..)) => self.parse_while_expression().map(|while_node| ExprNode {
				pos: while_node.pos,
				kind: ExprKind::While(while_node),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::KeywordFor(..)) => self.parse_for_expression().map(|for_node| ExprNode {
				pos: for_node.pos,
				kind: ExprKind::For(for_node),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::KeywordLet(..)) => self.parse_let_expression().map(|node| ExprNode {
				pos: node.pos,
				kind: ExprKind::Let(node),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::KeywordFun(..)) => self.parse_lambda().map(|lambda| ExprNode {
				pos: lambda.pos,
				kind: ExprKind::Lambda(lambda),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::Identifier(..)) => self.parse_identifier().map(|ident| ExprNode {
				pos: ident.pos,
				kind: ExprKind::Identifier(ident),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|literal| ExprNode {
				pos: literal.pos,
				kind: ExprKind::Literal(literal),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|literal| ExprNode {
				pos: literal.pos,
				kind: ExprKind::Literal(literal),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|literal| ExprNode {
				pos: literal.pos,
				kind: ExprKind::Literal(literal),
				resolved_type: ExprType::Unknown,
			}),
			Some(Token::HexDigits(..)) => self.parse_hex_number().map(|literal| ExprNode {
				pos: literal.pos,
				kind: ExprKind::Literal(literal),
				resolved_type: ExprType::Unknown,
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
					pos: (start, start),
					kind: ExprKind::UnaryOperation {
						op: operator,
						right: Box::new(rhs_expr),
					},
					resolved_type: ExprType::Unknown,
				})
			}
			_ => None,
		}?;

		loop {
			self.skip_line_breaks();

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

					let start = lhs_expr.pos.0;
					let end = args.last().expect("at least one arg").pos.1;

					lhs_expr = ExprNode {
						pos: (start, end),
						kind: ExprKind::Call(CallNode {
							pos: (start, end),
							callee: Box::new(lhs_expr),
							args,
						}),
						resolved_type: ExprType::Unknown,
					};
				} else {
					let op_pos = self.current_token_position();

					// advance past the operator token
					self.advance();

					let rhs_expr = self.parse_expression_with_binding_power(right_bp)?;

					if let Operator::IndexAccess = operator {
						// special case: the [ operator needs a closing ]
						expect_token_and_do!(self, Token::RightBracket, {
							self.advance();
						});
					}

					lhs_expr = ExprNode {
						pos: (lhs_expr.pos.0, rhs_expr.pos.1),
						kind: ExprKind::BinaryOperation {
							op: OperatorNode {
								pos: op_pos,
								kind: operator,
							},
							left: Box::new(lhs_expr),
							right: Box::new(rhs_expr),
						},
						resolved_type: ExprType::Unknown,
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
			pos: (start, end),
			name,
		})
	}

	fn parse_if_expression(&mut self) -> Option<IfNode> {
		let start = expect_token_and_do!(self, Token::KeywordIf, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		let condition = self.parse_expression()?;

		expect_token_and_do!(self, Token::KeywordIs, {
			self.advance();
		});

		let pattern = self.parse_pattern()?;

		expect_token_and_do!(self, Token::LeftBrace, {
			self.advance();
		});

		let body = self.parse_body_expressions()?;

		let end = expect_token_and_do!(self, Token::RightBrace, {
			let block_end = self.current_token_position().1;
			self.advance();
			block_end
		});

		Some(IfNode {
			pos: (start, end),
			condition: Box::new(condition),
			pattern,
			body,
		})
	}

	fn parse_when_expression(&mut self) -> Option<WhenNode> {
		let start = expect_token_and_do!(self, Token::KeywordWhen, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		let subject = self.parse_expression()?;

		self.skip_line_breaks();

		let mut cases = Vec::new();

		expect_token_and_do!(self, Token::KeywordIs, {});

		while current_token_is!(self, Token::KeywordIs) {
			let case_start = self.current_token_position().0;

			self.advance();

			let case_pattern = self.parse_pattern()?;

			expect_token_and_do!(self, Token::LeftBrace, {
				self.advance();
			});

			let case_body = self.parse_body_expressions()?;

			let case_end = expect_token_and_do!(self, Token::RightBrace, {
				let block_end = self.current_token_position().1;
				self.advance();
				block_end
			});

			self.skip_line_breaks();

			cases.push(CaseNode {
				pos: (case_start, case_end),
				pattern: case_pattern,
				body: case_body,
			})
		}

		let end = cases.last().unwrap().pos.1;

		Some(WhenNode {
			pos: (start, end),
			subject: Box::new(subject),
			cases,
		})
	}

	fn parse_while_expression(&mut self) -> Option<WhileNode> {
		let start = expect_token_and_do!(self, Token::KeywordWhile, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		let condition = self.parse_expression()?;

		expect_token_and_do!(self, Token::KeywordIs, {
			self.advance();
		});

		let pattern = self.parse_pattern()?;

		expect_token_and_do!(self, Token::LeftBrace, {
			self.advance();
		});

		let body = self.parse_body_expressions()?;

		let end = expect_token_and_do!(self, Token::RightBrace, {
			let block_end = self.current_token_position().1;
			self.advance();
			block_end
		});

		Some(WhileNode {
			pos: (start, end),
			condition: Box::new(condition),
			pattern,
			body,
		})
	}

	fn parse_for_expression(&mut self) -> Option<ForNode> {
		let start = expect_token_and_do!(self, Token::KeywordFor, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		let pattern = self.parse_pattern()?;

		expect_token_and_do!(self, Token::KeywordIn, {
			self.advance();
		});

		let data = self.parse_expression()?;

		expect_token_and_do!(self, Token::LeftBrace, {
			self.advance();
		});

		let body = self.parse_body_expressions()?;

		let end = expect_token_and_do!(self, Token::RightBrace, {
			let end = self.current_token_position().1;
			self.advance();
			end
		});

		Some(ForNode {
			pos: (start, end),
			data: Box::new(data),
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
						pos: (id_node.pos.0, arg_pattern.pos.1),
						kind: PatternKind::Constructor(id_node, Box::new(arg_pattern)),
					});
				}

				Some(PatternNode {
					pos: id_node.pos,
					kind: PatternKind::Identifier(id_node),
				})
			}

			Some(Token::LeftParen(start, _)) => {
				self.advance();

				let mut entries = Vec::new();

				loop {
					let next_token = self.tokenizer.peek();

					match (self.current_token, next_token) {
						(Some(Token::Identifier(..)), Some(Token::Colon(..))) => {
							// It's a labeled entry!
							let label = self.parse_identifier().unwrap();

							expect_token_and_do!(self, Token::Colon, { self.advance() });

							let pattern = match self.parse_pattern() {
								Some(pattern) => pattern,
								_ => break,
							};

							entries.push((Some(label), pattern));
						}

						_ => {
							// It's an unlabeled entry...
							let pattern = match self.parse_pattern() {
								Some(pattern) => pattern,
								_ => break,
							};

							entries.push((None, pattern));
						}
					}

					match self.current_token {
						Some(Token::Comma(..)) => self.advance(),
						_ => break,
					}
				}

				let end = expect_token_and_do!(self, Token::RightParen, {
					let (_, end) = self.current_token_position();
					self.advance();
					end
				});

				Some(PatternNode {
					pos: (start, end),
					kind: PatternKind::Tuple(entries),
				})
			}

			Some(Token::Underscore(start, end)) => {
				self.advance();

				Some(PatternNode {
					pos: (start, end),
					kind: PatternKind::Underscore,
				})
			}

			Some(Token::StringLiteral(..)) => self.parse_string().map(|expr_node| match expr_node.kind {
				ExprKind::Literal(literal) => PatternNode {
					pos: literal.pos,
					kind: PatternKind::Literal(literal),
				},
				ExprKind::Interpolation(parts) => PatternNode {
					pos: expr_node.pos,
					kind: PatternKind::Interpolation(parts),
				},
				_ => unreachable!(),
			}),

			Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| PatternNode {
				pos: lit_node.pos,
				kind: PatternKind::Literal(lit_node),
			}),

			// TODO: other kinds of digits here
			_ => None,
		}
	}

	fn parse_let_expression(&mut self) -> Option<LetNode> {
		let start = expect_token_and_do!(self, Token::KeywordLet, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		let name = match self.parse_identifier() {
			Some(node) => node,
			_ => todo!(),
		};

		expect_token_and_do!(self, Token::Equal, {
			self.advance();
		});

		let (end, value) = match self.parse_expression() {
			Some(node) => (node.pos.1, node),
			_ => {
				return self.error(ParseError {
					pos: self.current_token_position(),
					kind: ParseErrorKind::MissingRightHandSideOfAssignment,
				})
			}
		};

		Some(LetNode {
			pos: (start, end),
			name,
			value: Box::new(value),
		})
	}

	fn parse_list(&mut self) -> Option<ExprNode> {
		let start = expect_token_and_do!(self, Token::LeftBracket, {
			let pos = self.current_token_position();
			self.advance();
			pos.0
		});

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

		let end = expect_token_and_do!(self, Token::RightBracket, {
			let pos = self.current_token_position();
			self.advance();
			pos.1
		});

		Some(ExprNode {
			pos: (start, end),
			kind: ExprKind::List(elements),
			resolved_type: ExprType::Unknown,
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
					pos: (start, end),
					kind: ParseErrorKind::OverflowingIntegerLiteral,
				});
				return 0;
			}

			if let Some(next_i) = i.checked_mul(radix) {
				i = next_i;
			} else {
				self.error::<LiteralNode>(ParseError {
					pos: (start, end),
					kind: ParseErrorKind::OverflowingIntegerLiteral,
				});
				return 0;
			}
		}

		result
	}

	fn parse_parenthetical(&mut self) -> Option<ExprNode> {
		// "parentheticals" are a little tricky, because they could be a number of things:
		//  - "()" is an empty tuple
		//  - "(expr)" is an expression in parentheses (a grouping),
		//  - "(expr, expr, ...)" is an unlabeled tuple
		//  - "(ident: expr, ...)" is a labeled tuple

		let paren_start = expect_token_and_do!(self, Token::LeftParen, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		self.skip_line_breaks();

		let mut entries = Vec::new();

		while let Some(node) = self.parse_expression() {
			if current_token_is!(self, Token::Colon) {
				self.advance();

				match node.kind {
					ExprKind::Identifier(label) => {
						self.skip_line_breaks();

						if let Some(value) = self.parse_expression() {
							entries.push(TupleEntry(Some(label), value));
						} else {
							self.error::<ExprNode>(ParseError {
								pos: node.pos,
								kind: ParseErrorKind::MissingExpressionAfterLabelInTuple,
							});
						}
					}
					_ => {
						self.error::<ExprNode>(ParseError {
							pos: node.pos,
							kind: ParseErrorKind::MissingLabelInTuple,
						});
					}
				}
			} else {
				entries.push(TupleEntry(None, node));
			}

			self.skip_line_breaks();

			match self.current_token {
				Some(Token::Comma(..)) => {
					self.advance();
					self.skip_line_breaks();
				}
				_ => break,
			}
		}

		self.skip_line_breaks();

		let paren_end = expect_token_and_do!(self, Token::RightParen, {
			let pos = self.current_token_position();
			self.advance();
			pos.1
		});

		if entries.is_empty() {
			// If no expressions were found between the ()s, it's an empty tuple
			return Some(ExprNode {
				pos: (paren_start, paren_end),
				kind: ExprKind::EmptyTuple,
				resolved_type: ExprType::Unknown,
			});
		}

		if entries.len() == 1 {
			// If only one, unlabeled expression was found, it's a grouping
			if let Some(TupleEntry(None, first_expr)) = entries.pop() {
				return Some(ExprNode {
					pos: (paren_start, paren_end),
					kind: ExprKind::Grouping(Box::new(first_expr)),
					resolved_type: ExprType::Unknown,
				});
			}
		}

		// Otherwise, it's a tuple with multiple entries, some of which may
		// have labels:
		Some(ExprNode {
			pos: (paren_start, paren_end),
			kind: ExprKind::Tuple(entries),
			resolved_type: ExprType::Unknown,
		})
	}

	fn parse_regular_expression(&mut self) -> Option<ExprNode> {
		let start = expect_token_and_do!(self, Token::Backtick, {
			let (start, _) = self.current_token_position();
			self.advance();
			start
		});

		self.skip_line_breaks();

		let maybe_reg_expr_node = self.parse_regular_expression_body();

		self.skip_line_breaks();

		let end = expect_token_and_do!(self, Token::Backtick, {
			let (_, end) = self.current_token_position();
			self.advance();
			end
		});

		let regex = match maybe_reg_expr_node {
			Some(expr) => expr,
			None => {
				return self.error(ParseError {
					pos: (start, end),
					kind: ParseErrorKind::EmptyRegularExpression,
				})
			}
		};

		Some(ExprNode {
			pos: (start, end),
			kind: ExprKind::Regex(regex),
			resolved_type: ExprType::Unknown,
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

		let start = other_terms.first().unwrap().pos.0;
		let end = other_terms.last().unwrap().pos.1;

		Some(RegexNode {
			pos: (start, end),
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
						pos: (start, end),
						kind: RegexKind::CharacterClass(name),
					}
				}

				Some(Token::StringLiteral(start, end)) => {
					self.advance();

					let value = read_string_with_escapes!(self, start, end);

					RegexNode {
						pos: (start, end),
						kind: RegexKind::Literal(value),
					}
				}

				Some(Token::LeftParen(start, end)) => {
					self.advance();

					let expr = match self.parse_regular_expression_body() {
						Some(expr) => expr,
						None => {
							return self.error(ParseError {
								pos: (start, end),
								kind: ParseErrorKind::EmptyRegularExpressionGroup,
							})
						}
					};

					expect_token_and_do!(self, Token::RightParen, { self.advance() });

					RegexNode {
						pos: (start, end),
						kind: RegexKind::Grouping(Box::new(expr)),
					}
				}

				Some(Token::LeftAngle(start, end)) => {
					self.advance();

					let name = expect_token_and_do!(self, Token::Identifier, {
						let (start, end) = self.current_token_position();
						let name = read_string!(self, start, end);
						self.advance();
						name
					});

					expect_token_and_do!(self, Token::Colon, { self.advance() });

					let expr = match self.parse_regular_expression_body() {
						Some(expr) => expr,
						None => {
							return self.error(ParseError {
								pos: (start, end),
								kind: ParseErrorKind::EmptyRegularExpressionGroup,
							})
						}
					};

					expect_token_and_do!(self, Token::RightAngle, { self.advance() });

					RegexNode {
						pos: (start, end),
						kind: RegexKind::NamedCapture(name, Box::new(expr)),
					}
				}

				_ => break,
			};

			let modified_part = match self.current_token {
				Some(Token::Star(_, end)) => {
					self.advance();

					RegexNode {
						pos: (part.pos.0, end),
						kind: RegexKind::ZeroOrMore(Box::new(part)),
					}
				}

				Some(Token::Plus(_, end)) => {
					self.advance();

					RegexNode {
						pos: (part.pos.0, end),
						kind: RegexKind::OneOrMore(Box::new(part)),
					}
				}

				Some(Token::Question(_, end)) => {
					self.advance();

					RegexNode {
						pos: (part.pos.0, end),
						kind: RegexKind::OneOrZero(Box::new(part)),
					}
				}

				Some(Token::LeftBrace(_, _)) => {
					self.advance();

					let mut min_count = None;
					let mut max_count = None;
					let mut has_comma = false;

					if current_token_is!(self, Token::DecimalDigits) {
						let (start, end) = self.current_token_position();
						let value = self.parse_numeric_literal(start, end, 10) as usize;
						min_count = Some(value);
						self.advance();
					}

					if current_token_is!(self, Token::Comma) {
						has_comma = true;

						self.advance();

						if current_token_is!(self, Token::DecimalDigits) {
							let (start, end) = self.current_token_position();
							let value = self.parse_numeric_literal(start, end, 10) as usize;
							max_count = Some(value);
							self.advance();
						}
					}

					let end = expect_token_and_do!(self, Token::RightBrace, {
						let (_, end) = self.current_token_position();
						self.advance();
						end
					});

					match (min_count, max_count, has_comma) {
						(Some(min), None, true) => RegexNode {
							pos: (part.pos.0, end),
							kind: RegexKind::AtLeastCount(Box::new(part), min),
						},

						(None, Some(max), true) => RegexNode {
							pos: (part.pos.0, end),
							kind: RegexKind::AtMostCount(Box::new(part), max),
						},

						(Some(min), None, false) => RegexNode {
							pos: (part.pos.0, end),
							kind: RegexKind::ExactCount(Box::new(part), min),
						},

						(Some(min), Some(max), true) => {
							if min > max {
								self.error::<RegexNode>(ParseError {
									pos: (part.pos.0, end),
									kind: ParseErrorKind::InvalidRegularExpressionCountModifier,
								});
							}

							RegexNode {
								pos: (part.pos.0, end),
								kind: RegexKind::RangeCount(Box::new(part), min, max),
							}
						}

						_ => {
							return self.error(ParseError {
								pos: (part.pos.0, end),
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
			pos: (0, 0),
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
		let doc_comment_lines_start = self.line_breaks.len();

		self.skip_line_breaks();

		let doc_comment_lines_end = self.line_breaks.len();

		expect_token_and_do!(self, Token::Equal, {
			self.advance();
		});

		let value = self.parse_expression()?;

		let end = value.pos.1;

		self.skip_line_breaks();

		Some(DefinitionNode {
			name,
			pos: (start, end),
			doc_comment_lines: (doc_comment_lines_start..doc_comment_lines_end),
			kind: DefinitionKind::Expr(value),
		})
	}

	fn parse_string(&mut self) -> Option<ExprNode> {
		let (start, end) = expect_token_and_do!(self, Token::StringLiteral, {
			let pos = self.current_token_position();
			self.advance();
			pos
		});

		let value = read_string_with_escapes!(self, start, end);

		let literal = LiteralNode {
			pos: (start, end),
			kind: LiteralKind::Str(value),
		};

		let expr_node = ExprNode {
			pos: (start, end),
			kind: ExprKind::Literal(literal),
			resolved_type: ExprType::String,
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

				expect_token_and_do!(self, Token::InterpolationEnd, {
					self.advance();
				});

				expect_token_and_do!(self, Token::StringLiteral, {
					let (start, end) = self.current_token_position();

					interpolation_end = end;

					let value = read_string_with_escapes!(self, start, end);

					parts.push(ExprNode {
						pos: (start, end),
						kind: ExprKind::Literal(LiteralNode {
							pos: (start, end),
							kind: LiteralKind::Str(value),
						}),
						resolved_type: ExprType::String,
					});

					self.advance()
				})
			}

			return Some(ExprNode {
				pos: (start, interpolation_end),
				kind: ExprKind::Interpolation(parts),
				resolved_type: ExprType::String,
			});
		}

		Some(expr_node)
	}
}
