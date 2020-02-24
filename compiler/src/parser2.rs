use crate::ast2::*;
use crate::tokens::Token;

macro_rules! current_token_is {
  ($self:ident, $tokType:path) => {
    match $self.current_token() {
      Some(&$tokType(..)) => true,
      _ => false,
    }
  };
}

macro_rules! expect_token_and_do {
  ($self:ident, $tokType:path, $block:tt) => {
    match $self.current_token() {
      Some(&$tokType(..)) => $block,
      Some(&tok) => {
        return $self.error(ParseError {
          pos: tok.get_location(),
          kind: ParseErrorKind::UnexpectedToken(tok.clone()),
        })
      }
      None => {
        return $self.error(ParseError {
          pos: ($self.source.len(), $self.source.len()),
          kind: ParseErrorKind::UnexpectedEOF,
        })
      }
    }
  };
}

macro_rules! read_string {
  ($self:ident, $start:expr, $end:expr) => {
    String::from_utf8($self.source[$start..$end].to_vec()).expect("not utf-8");
  };
}

pub struct Parser<'a> {
  source: &'a Vec<u8>,
  tokens: &'a Vec<Token>,
  index: usize,
  errors: Vec<ParseError>,
  next_node_id: usize,
}

impl<'a> Parser<'a> {
  pub fn new(source: &'a Vec<u8>, tokens: &'a Vec<Token>) -> Parser<'a> {
    return Parser {
      source,
      tokens,
      index: 0,
      errors: Vec::new(),
      next_node_id: 0,
    };
  }

  pub fn parse_module(&mut self) -> (ModuleNode, Vec<ParseError>) {
    let mut body = Vec::new();

    loop {
      self.skip_line_breaks();

      match self.parse_top_level_statement() {
        Some(statement) => body.push(statement),
        _ => break,
      }
    }

    let start = body.first().map_or(0, |node| node.pos.0);
    let end = body.last().map_or(0, |node| node.pos.1);

    let module_node = ModuleNode {
      id: self.next_id(),
      pos: (start, end),
      body,
    };

    (module_node, self.errors.clone())
  }

  fn advance(&mut self) {
    self.index += 1;
  }

  fn skip_line_breaks(&mut self) {
    while current_token_is!(self, Token::LineBreak) {
      self.advance()
    }
  }

  fn current_token(&self) -> Option<&Token> {
    self.tokens.get(self.index)
  }

  fn next_token(&self) -> Option<&Token> {
    self.tokens.get(self.index + 1)
  }

  fn current_token_location(&self) -> (usize, usize) {
    self.current_token().unwrap().get_location()
  }

  fn next_id(&mut self) -> usize {
    let id = self.next_node_id;
    self.next_node_id += 1;
    id
  }

  fn error<A>(&mut self, err: ParseError) -> Option<A> {
    self.errors.push(err);
    None
  }

  fn parse_array_or_dict(&mut self) -> Option<ExprNode> {
    let start = expect_token_and_do!(self, Token::LeftBracket, {
      let pos = self.current_token_location();
      self.advance();
      pos.0
    });

    let mut array_elements = Vec::new();
    let mut dict_entries = Vec::new();

    while let Some(expr) = self.parse_expression() {
      if current_token_is!(self, Token::Colon) {
        if !array_elements.is_empty() {
          self.error::<ExprNode>(ParseError {
            pos: self.current_token_location(),
            kind: ParseErrorKind::UnexpectedDictValueInArray,
          });
        }

        self.advance();

        match self.parse_expression() {
          Some(val) => dict_entries.push((expr, val)),
          _ => {
            return self.error(ParseError {
              pos: self.current_token_location(),
              kind: ParseErrorKind::MissingDictValue,
            })
          }
        }
      } else {
        if !dict_entries.is_empty() {
          self.error::<ExprNode>(ParseError {
            pos: self.current_token_location(),
            kind: ParseErrorKind::MissingDictValue,
          });
        }

        array_elements.push(expr);
      }

      if current_token_is!(self, Token::Comma) {
        self.advance()
      } else {
        break;
      }
    }

    let end = expect_token_and_do!(self, Token::RightBracket, {
      let pos = self.current_token_location();
      self.advance();
      pos.1
    });

    let kind = if dict_entries.is_empty() {
      ExprKind::Array(array_elements)
    } else {
      ExprKind::Dict(dict_entries)
    };

    Some(ExprNode {
      id: self.next_id(),
      pos: (start, end),
      kind,
    })
  }

  fn parse_binary_number(&mut self) -> Option<LitNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::BinaryDigits, {
      let (start, end) = self.current_token_location();
      (start, end, self.parse_numeric_literal(start, end, 2))
    });

    self.advance();

    Some(LitNode {
      id: self.next_id(),
      kind: LitKind::IntBinary(value),
      pos: (start, end),
    })
  }

  fn parse_block(&mut self) -> Option<ExprNode> {
    let block_start = expect_token_and_do!(self, Token::LeftBrace, {
      let (start, _) = self.current_token_location();
      self.advance();
      start
    });

    self.skip_line_breaks();

    let mut params = Vec::new();
    let mut body = Vec::new();

    while current_token_is!(self, Token::IdentifierLower) {
      if params.is_empty() {
        // If no params yet, and the next token isn't a , or a =>, assume
        // there are no params in this block and break out of the loop
        match self.next_token() {
          Some(&Token::Comma(..)) => {}
          Some(&Token::DoubleArrow(..)) => {}
          _ => break,
        }
      }

      let param = self.parse_identifier().unwrap();

      params.push(param);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        Some(&Token::DoubleArrow(..)) => {
          self.advance();
          break;
        }
        _ => todo!(),
      }
    }

    self.skip_line_breaks();

    while let Some(node) = self.parse_statement() {
      body.push(node);
    }

    self.skip_line_breaks();

    let block_end = expect_token_and_do!(self, Token::RightBrace, {
      let pos = self.current_token_location();
      self.advance();
      pos.1
    });

    Some(ExprNode {
      id: self.next_id(),
      pos: (block_start, block_end),
      kind: ExprKind::Block { params, body },
    })
  }

  fn parse_binary_operation(&mut self, last_term: ExprNode) -> Option<ExprNode> {
    let op_node = expect_token_and_do!(self, Token::Operator, {
      let (start, end) = self.current_token_location();
      let name = read_string!(self, start, end);
      self.advance();

      Box::new(OperatorNode {
        id: self.next_id(),
        pos: (start, end),
        name,
      })
    });

    let (end, next_term) = match self.parse_operator_branch() {
      Some(term) => (term.pos.1, Box::new(term)),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingExpressionAfterDot,
        })
      }
    };

    Some(ExprNode {
      id: self.next_id(),
      pos: (last_term.pos.0, end),
      kind: ExprKind::BinaryOperation {
        left: Box::new(last_term),
        op: op_node,
        right: next_term,
      },
    })
  }

  fn parse_call(&mut self, last_expr: ExprNode) -> Option<ExprNode> {
    expect_token_and_do!(self, Token::LeftParen, { self.advance() });

    let mut args = Vec::new();
    let end;

    if current_token_is!(self, Token::RightParen) {
      end = self.current_token_location().1;
      self.advance()
    } else {
      while let Some(expr) = self.parse_expression() {
        args.push(expr);

        if current_token_is!(self, Token::Comma) {
          self.advance()
        } else {
          break;
        }
      }

      expect_token_and_do!(self, Token::RightParen, {
        end = self.current_token_location().1;
        self.advance()
      });
    }

    Some(ExprNode {
      id: self.next_id(),
      pos: (last_expr.pos.0, end),
      kind: ExprKind::Call {
        callee: Box::new(last_expr),
        args,
      },
    })
  }

  fn parse_chain(&mut self, last_expr: ExprNode) -> Option<ExprNode> {
    expect_token_and_do!(self, Token::Dot, { self.advance() });

    let (end, next_expr) = match self.parse_term() {
      Some(term) => (term.pos.1, Box::new(term)),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingExpressionAfterDot,
        })
      }
    };

    Some(ExprNode {
      id: self.next_id(),
      pos: (last_expr.pos.0, end),
      kind: ExprKind::Chain {
        obj: Box::new(last_expr),
        prop: next_expr,
      },
    })
  }

  fn parse_decimal_number(&mut self) -> Option<LitNode> {
    let (start, end) = expect_token_and_do!(self, Token::DecimalDigits, {
      let pos = self.current_token_location();
      self.advance();
      pos
    });

    if current_token_is!(self, Token::Dot) {
      self.advance();

      expect_token_and_do!(self, Token::DecimalDigits, {
        let (_, end) = self.current_token_location();

        self.advance();

        let str_value = read_string!(self, start, end);
        let float_value = str_value.parse::<f64>().unwrap();

        return Some(LitNode {
          id: self.next_id(),
          kind: LitKind::FloatDecimal(float_value),
          pos: (start, end),
        });
      });
    }

    let value = self.parse_numeric_literal(start, end, 10);

    Some(LitNode {
      id: self.next_id(),
      kind: LitKind::IntDecimal(value),
      pos: (start, end),
    })
  }

  fn parse_definition(&mut self) -> Option<DefNode> {
    let start = match self.current_token() {
      Some(&Token::KeywordDef(start, _)) => {
        self.advance();
        start
      }
      _ => unreachable!(),
    };

    let kind = match self.parse_definition_kind() {
      Some(kind_node) => kind_node,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::UnexpectedToken(*self.current_token().unwrap()),
        })
      }
    };

    let return_type = if current_token_is!(self, Token::Arrow) {
      self.advance();

      match self.parse_type_expression() {
        Some(type_node) => Some(type_node),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_location(),
            kind: ParseErrorKind::MissingReturnType,
          })
        }
      }
    } else {
      None
    };

    self.skip_line_breaks();

    let (params, body, end) = match self.parse_block() {
      Some(ExprNode {
        kind: ExprKind::Block { params, body },
        pos,
        ..
      }) => (params, body, pos.1),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingDefinitionBody,
        })
      }
    };

    Some(DefNode {
      id: self.next_id(),
      pos: (start, end),
      kind,
      return_type,
      params,
      body,
    })
  }

  fn parse_definition_kind(&mut self) -> Option<DefKind> {
    match self.current_token() {
      Some(&Token::IdentifierLower(..)) => self.parse_definition_kind_function(),
      Some(&Token::LeftParen(..)) => self.parse_definition_kind_receiver(),
      Some(&Token::Operator(..)) => self.parse_definition_kind_unary_op(),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingType,
        })
      }
    }
  }

  fn parse_definition_kind_function(&mut self) -> Option<DefKind> {
    let ident = match self.parse_identifier() {
      Some(ident_node) => ident_node,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingIdentifier,
        })
      }
    };

    expect_token_and_do!(self, Token::LeftParen, {
      self.advance();
    });

    let mut type_params = Vec::new();

    while let Some(type_node) = self.parse_type_expression() {
      type_params.push(type_node);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    expect_token_and_do!(self, Token::RightParen, {
      self.advance();
    });

    Some(DefKind::Function {
      parts: vec![(Box::new(ident), type_params)],
    })
  }

  fn parse_definition_kind_receiver(&mut self) -> Option<DefKind> {
    expect_token_and_do!(self, Token::LeftParen, {
      self.advance();
    });

    let receiver = match self.parse_type_expression() {
      Some(node) => Box::new(node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingType,
        })
      }
    };

    expect_token_and_do!(self, Token::RightParen, {
      self.advance();
    });

    if current_token_is!(self, Token::LeftBracket) {
      self.advance();

      let type_node = match self.parse_type_expression() {
        Some(node) => Box::new(node),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_location(),
            kind: ParseErrorKind::MissingType,
          })
        }
      };

      expect_token_and_do!(self, Token::RightBracket, {
        self.advance();
      });

      return Some(DefKind::Index {
        receiver,
        index: type_node,
      });
    }

    if current_token_is!(self, Token::Operator) {
      let op_node = expect_token_and_do!(self, Token::Operator, {
        let (start, end) = self.current_token_location();
        let name = read_string!(self, start, end);
        self.advance();

        Box::new(OperatorNode {
          id: self.next_id(),
          pos: (start, end),
          name,
        })
      });

      expect_token_and_do!(self, Token::LeftParen, {
        self.advance();
      });

      let type_node = match self.parse_type_expression() {
        Some(node) => Box::new(node),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_location(),
            kind: ParseErrorKind::MissingType,
          })
        }
      };

      expect_token_and_do!(self, Token::RightParen, {
        self.advance();
      });

      return Some(DefKind::BinaryOperator {
        left: receiver,
        op: op_node,
        right: type_node,
      });
    }

    expect_token_and_do!(self, Token::Dot, {
      self.advance();
    });

    let ident = match self.parse_identifier() {
      Some(ident_node) => Box::new(ident_node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingIdentifier,
        })
      }
    };

    expect_token_and_do!(self, Token::LeftParen, {
      self.advance();
    });

    let mut type_params = Vec::new();

    while let Some(type_node) = self.parse_type_expression() {
      type_params.push(type_node);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    expect_token_and_do!(self, Token::RightParen, {
      self.advance();
    });

    Some(DefKind::Method {
      receiver,
      parts: vec![(ident, type_params)],
    })
  }

  fn parse_definition_kind_unary_op(&mut self) -> Option<DefKind> {
    let op_node = expect_token_and_do!(self, Token::Operator, {
      let (start, end) = self.current_token_location();
      let name = read_string!(self, start, end);
      self.advance();

      Box::new(OperatorNode {
        id: self.next_id(),
        pos: (start, end),
        name,
      })
    });

    expect_token_and_do!(self, Token::LeftParen, {
      self.advance();
    });

    let type_node = match self.parse_type_expression() {
      Some(node) => Box::new(node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingType,
        })
      }
    };

    expect_token_and_do!(self, Token::RightParen, {
      self.advance();
    });

    Some(DefKind::UnaryOperator {
      op: op_node,
      right: type_node,
    })
  }

  fn parse_expression(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_operator_branch();

    loop {
      if expr.is_some() {
        match self.current_token() {
          Some(&Token::Operator(..)) => {
            expr = self.parse_binary_operation(expr.unwrap());
            continue;
          }
          _ => {}
        }
      }

      break;
    }

    expr
  }

  fn parse_hex_number(&mut self) -> Option<LitNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::HexDigits, {
      let (start, end) = self.current_token_location();
      (start, end, self.parse_numeric_literal(start, end, 16))
    });

    self.advance();

    Some(LitNode {
      id: self.next_id(),
      kind: LitKind::IntHex(value),
      pos: (start, end),
    })
  }

  fn parse_identifier(&mut self) -> Option<IdentNode> {
    let (start, end) = expect_token_and_do!(self, Token::IdentifierLower, {
      let (start, end) = self.current_token_location();
      self.advance();
      (start, end)
    });

    let name = read_string!(self, start, end);

    Some(IdentNode {
      id: self.next_id(),
      pos: (start, end),
      name,
    })
  }

  fn parse_index(&mut self, last_expr: ExprNode) -> Option<ExprNode> {
    expect_token_and_do!(self, Token::LeftBracket, { self.advance() });

    let index_node;
    let end;

    if let Some(node) = self.parse_expression() {
      index_node = Box::new(node);

      expect_token_and_do!(self, Token::RightBracket, {
        end = self.current_token_location().1;
        self.advance()
      });
    } else {
      return self.error(ParseError {
        pos: self.current_token_location(),
        kind: ParseErrorKind::MissingIndexBetweenBrackets,
      });
    }

    Some(ExprNode {
      id: self.next_id(),
      pos: (last_expr.pos.0, end),
      kind: ExprKind::Index(Box::new(last_expr), index_node),
    })
  }

  fn parse_let_statement(&mut self) -> Option<LetNode> {
    let start = expect_token_and_do!(self, Token::KeywordLet, {
      let (start, _) = self.current_token_location();
      self.advance();
      start
    });

    let pattern = match self.parse_pattern() {
      Some(node) => node,
      _ => todo!(),
    };

    expect_token_and_do!(self, Token::Equals, {
      self.advance();
      self.skip_line_breaks();
    });

    let (end, value) = match self.parse_expression() {
      Some(node) => (node.pos.1, node),
      _ => todo!(),
    };

    Some(LetNode {
      id: self.next_id(),
      pos: (start, end),
      pattern,
      value,
    })
  }

  fn parse_match(&mut self) -> Option<ExprNode> {
    let start = match self.current_token() {
      Some(&Token::KeywordMatch(start, _)) => {
        self.advance();
        start
      }
      _ => unreachable!(),
    };

    let subject = match self.parse_expression() {
      Some(node) => node,
      _ => todo!(),
    };

    self.skip_line_breaks();

    let mut cases = Vec::new();
    let mut match_end = start;

    while let Some(&Token::Pipe(case_start, _)) = self.current_token() {
      self.advance();

      let pattern = match self.parse_pattern() {
        Some(node) => node,
        _ => todo!(),
      };

      match self.current_token() {
        Some(&Token::DoubleArrow(..)) => self.advance(),
        _ => todo!(),
      };

      self.skip_line_breaks();

      let (case_end, body) = match self.parse_expression() {
        Some(node) => (node.pos.1, node),
        other => return other,
      };

      self.skip_line_breaks();
      match_end = case_end;

      cases.push(MatchCaseNode {
        id: self.next_id(),
        pos: (case_start, case_end),
        pattern,
        body,
      });
    }

    if cases.is_empty() {
      self.error::<ExprNode>(ParseError {
        pos: (start, match_end),
        kind: ParseErrorKind::MissingMatchCases,
      });
    }

    Some(ExprNode {
      id: self.next_id(),
      pos: (start, match_end),
      kind: ExprKind::Match(MatchNode {
        id: self.next_id(),
        pos: (start, match_end),
        subject: Box::new(subject),
        cases,
      }),
    })
  }

  fn parse_numeric_literal(&self, start: usize, end: usize, radix: i128) -> i128 {
    let mut result: i128 = 0;
    let mut i: i128 = 1;

    for byte in self.source[start..end].iter().rev() {
      let byte_value = match byte {
        b'0'..=b'9' => byte - 48,
        _ => unreachable!(),
      };

      result += (byte_value as i128) * i;
      i *= radix;
    }

    result
  }

  fn parse_octal_number(&mut self) -> Option<LitNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::OctalDigits, {
      let (start, end) = self.current_token_location();
      (start, end, self.parse_numeric_literal(start, end, 8))
    });

    self.advance();

    Some(LitNode {
      id: self.next_id(),
      kind: LitKind::IntOctal(value),
      pos: (start, end),
    })
  }

  fn parse_operator_branch(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_term();

    loop {
      if expr.is_some() {
        match self.current_token() {
          Some(&Token::Dot(..)) => {
            expr = self.parse_chain(expr.unwrap());
            continue;
          }
          Some(&Token::LeftParen(..)) => {
            expr = self.parse_call(expr.unwrap());
            continue;
          }
          Some(&Token::LeftBracket(..)) => {
            expr = self.parse_index(expr.unwrap());
            continue;
          }
          _ => {}
        }
      }

      break;
    }

    expr
  }

  fn parse_pattern(&mut self) -> Option<PatternNode> {
    self.parse_identifier().map(|id_node| PatternNode {
      id: self.next_id(),
      pos: id_node.pos,
      kind: PatternKind::Ident(id_node),
    })
  }

  fn parse_parenthetical(&mut self) -> Option<ExprNode> {
    let paren_start = expect_token_and_do!(self, Token::LeftParen, {
      let (start, _) = self.current_token_location();
      self.advance();
      start
    });

    let mut first_expr = None;
    let mut other_exprs = Vec::new();

    while let Some(node) = self.parse_expression() {
      if first_expr.is_none() {
        first_expr = Some(node)
      } else {
        other_exprs.push(node);
      }

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    self.skip_line_breaks();

    let paren_end = match self.current_token() {
      Some(&Token::RightParen(_, end)) => end,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::UnclosedParentheses,
        })
      }
    };

    self.advance();

    if first_expr.is_none() {
      return Some(ExprNode {
        id: self.next_id(),
        pos: (paren_start, paren_end),
        kind: ExprKind::EmptyTuple,
      });
    }

    if other_exprs.is_empty() {
      return Some(ExprNode {
        id: self.next_id(),
        pos: (paren_start, paren_end),
        kind: ExprKind::Grouping(Box::new(first_expr.unwrap())),
      });
    }

    other_exprs.insert(0, first_expr.unwrap());

    Some(ExprNode {
      id: self.next_id(),
      pos: (paren_start, paren_end),
      kind: ExprKind::Tuple(other_exprs),
    })
  }

  fn parse_statement(&mut self) -> Option<StatementNode> {
    match self.current_token() {
      Some(&Token::KeywordLet(..)) => self.parse_let_statement().map(|let_node| StatementNode {
        id: self.next_id(),
        pos: let_node.pos,
        kind: StatementKind::Let(let_node),
      }),
      _ => self.parse_expression().map(|expr_node| StatementNode {
        id: self.next_id(),
        pos: expr_node.pos,
        kind: StatementKind::Expr(expr_node),
      }),
    }
  }

  fn parse_string(&mut self) -> Option<ExprNode> {
    let (start, end) = expect_token_and_do!(self, Token::StringLiteral, {
      let pos = self.current_token_location();
      self.advance();
      pos
    });

    let value = read_string!(self, start, end);

    let lit_node = LitNode {
      id: self.next_id(),
      pos: (start, end),
      kind: LitKind::Str(value),
    };

    let expr_node = ExprNode {
      id: self.next_id(),
      pos: (start, end),
      kind: ExprKind::Literal(lit_node),
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
          let (start, end) = self.current_token_location();

          interpolation_end = end;

          let value = read_string!(self, start, end);

          parts.push(ExprNode {
            id: self.next_id(),
            pos: (start, end),
            kind: ExprKind::Literal(LitNode {
              id: self.next_id(),
              pos: (start, end),
              kind: LitKind::Str(value),
            }),
          });

          self.advance()
        })
      }

      return Some(ExprNode {
        id: self.next_id(),
        pos: (start, interpolation_end),
        kind: ExprKind::Interpolation(parts),
      });
    }

    Some(expr_node)
  }

  fn parse_term(&mut self) -> Option<ExprNode> {
    match self.current_token() {
      Some(&Token::LeftBrace(..)) => self.parse_block(),
      Some(&Token::LeftBracket(..)) => self.parse_array_or_dict(),
      Some(&Token::StringLiteral(..)) => self.parse_string(),
      Some(&Token::KeywordMatch(..)) => self.parse_match(),
      Some(&Token::IdentifierLower(..)) | Some(&Token::IdentifierUpper(..)) => self
        .parse_identifier()
        .map(|id_node| match self.current_token() {
          Some(&Token::Equals(..)) => {
            self.advance();

            let expr = self.parse_expression().unwrap();

            ExprNode {
              id: self.next_id(),
              pos: (id_node.pos.0, expr.pos.1),
              kind: ExprKind::Assignment {
                left: Box::new(id_node),
                right: Box::new(expr),
              },
            }
          }
          _ => ExprNode {
            id: self.next_id(),
            pos: id_node.pos,
            kind: ExprKind::Identifier(id_node),
          },
        }),
      Some(&Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| ExprNode {
        id: self.next_id(),
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
      }),
      Some(&Token::HexDigits(..)) => self.parse_hex_number().map(|lit_node| ExprNode {
        id: self.next_id(),
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
      }),
      Some(&Token::OctalDigits(..)) => self.parse_octal_number().map(|lit_node| ExprNode {
        id: self.next_id(),
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
      }),
      Some(&Token::BinaryDigits(..)) => self.parse_binary_number().map(|lit_node| ExprNode {
        id: self.next_id(),
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
      }),
      Some(&Token::LeftParen(..)) => self.parse_parenthetical(),
      Some(&Token::Operator(..)) => self.parse_unary_operation(),
      _ => None,
    }
  }

  fn parse_top_level_statement(&mut self) -> Option<TopLevelStatementNode> {
    match self.current_token() {
      Some(&Token::KeywordLet(..)) => {
        self
          .parse_let_statement()
          .map(|let_node| TopLevelStatementNode {
            id: self.next_id(),
            pos: let_node.pos,
            kind: TopLevelStatementKind::Let(let_node),
          })
      }
      Some(&Token::KeywordDef(..)) => {
        self
          .parse_definition()
          .map(|def_node| TopLevelStatementNode {
            id: self.next_id(),
            pos: def_node.pos,
            kind: TopLevelStatementKind::Def(def_node),
          })
      }
      _ => self
        .parse_expression()
        .map(|expr_node| TopLevelStatementNode {
          id: self.next_id(),
          pos: expr_node.pos,
          kind: TopLevelStatementKind::Expr(expr_node),
        }),
    }
  }

  fn parse_type_block(&mut self) -> Option<TypeNode> {
    let start = expect_token_and_do!(self, Token::LeftBrace, {
      let pos = self.current_token_location();
      self.advance();
      pos.0
    });

    let mut param_types = Vec::new();

    while let Some(type_node) = self.parse_type_expression() {
      param_types.push(type_node);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    let return_type = if current_token_is!(self, Token::Arrow) {
      self.advance();

      match self.parse_type_expression() {
        Some(type_node) => Some(type_node),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_location(),
            kind: ParseErrorKind::MissingReturnType,
          })
        }
      }
    } else {
      if param_types.len() != 1 {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingReturnType,
        });
      }

      param_types.pop()
    };

    let end = expect_token_and_do!(self, Token::RightBrace, {
      let pos = self.current_token_location();
      self.advance();
      pos.1
    });

    Some(TypeNode {
      id: self.next_id(),
      pos: (start, end),
      kind: TypeKind::Block(param_types, Box::new(return_type.unwrap())),
    })
  }

  fn parse_type_expression(&mut self) -> Option<TypeNode> {
    match self.current_token() {
      Some(&Token::IdentifierUpper(..)) => self.parse_type_identifier(),
      Some(&Token::LeftParen(..)) => self.parse_type_tuple(),
      Some(&Token::LeftBrace(..)) => self.parse_type_block(),
      _ => None,
    }
  }

  fn parse_type_identifier(&mut self) -> Option<TypeNode> {
    let (start, end) = expect_token_and_do!(self, Token::IdentifierUpper, {
      let pos = self.current_token_location();
      self.advance();
      pos
    });

    let ident = IdentNode {
      id: self.next_id(),
      pos: (start, end),
      name: read_string!(self, start, end),
    };

    if current_token_is!(self, Token::LeftParen) {
      self.advance();

      let mut type_params = Vec::new();

      while let Some(type_node) = self.parse_type_expression() {
        type_params.push(type_node);

        match self.current_token() {
          Some(&Token::Comma(..)) => self.advance(),
          _ => break,
        }
      }

      let params_end = expect_token_and_do!(self, Token::RightParen, {
        let pos = self.current_token_location();
        self.advance();
        pos.1
      });

      return Some(TypeNode {
        id: self.next_id(),
        pos: (start, params_end),
        kind: TypeKind::Generic(ident, type_params),
      });
    }

    Some(TypeNode {
      id: self.next_id(),
      pos: (start, end),
      kind: TypeKind::Ident(ident),
    })
  }

  fn parse_type_tuple(&mut self) -> Option<TypeNode> {
    let start = expect_token_and_do!(self, Token::LeftParen, {
      let pos = self.current_token_location();
      self.advance();
      pos.0
    });

    let mut entries = Vec::new();

    while let Some(type_node) = self.parse_type_expression() {
      entries.push(type_node);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    let end = expect_token_and_do!(self, Token::RightParen, {
      let pos = self.current_token_location();
      self.advance();
      pos.1
    });

    Some(TypeNode {
      id: self.next_id(),
      pos: (start, end),
      kind: TypeKind::Tuple(entries),
    })
  }

  fn parse_unary_operation(&mut self) -> Option<ExprNode> {
    let op_node = expect_token_and_do!(self, Token::Operator, {
      let pos = self.current_token_location();
      let name = read_string!(self, pos.0, pos.1);
      self.advance();

      Box::new(OperatorNode {
        id: self.next_id(),
        pos,
        name,
      })
    });

    let expr_node = match self.parse_expression() {
      Some(node) => Box::new(node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_location(),
          kind: ParseErrorKind::MissingExpressionAfterOperator,
        })
      }
    };

    Some(ExprNode {
      id: self.next_id(),
      pos: (op_node.pos.0, expr_node.pos.1),
      kind: ExprKind::UnaryOperation {
        op: op_node,
        right: expr_node,
      },
    })
  }
}
