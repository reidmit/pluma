use crate::ast::{Node, Node::*, NodeType, NumericValue};
use crate::errors::ParseError;
use crate::parser::{ParseError::*, ParseResult::*};
use crate::tokens::Token;

pub struct Parser<'a> {
  source: &'a Vec<u8>,
  tokens: &'a Vec<Token>,
  index: usize,
  imports: Vec<Node>,
  nodes: Vec<Node>,
}

#[derive(Debug, Clone)]
enum ParseResult {
  Parsed(Node),
  EOF,
  Error(ParseError),
}

fn ungroup(node: Node) -> Node {
  match node {
    Grouping { expr, .. } => *expr,
    otherwise => otherwise,
  }
}

macro_rules! current_token_is {
  ($self:ident, $tokType:path) => {
    match $self.current_token() {
      Some(&$tokType(..)) => true,
      _ => false,
    }
  };
}

macro_rules! next_token_is {
  ($self:ident, $tokType:path) => {
    match $self.next_token() {
      Some(&$tokType(..)) => true,
      _ => false,
    }
  };
}

macro_rules! expect_token_and_do {
  ($self:ident, $tokType:path, $block:tt) => {
    match $self.current_token() {
      Some(&$tokType(..)) => $block,
      Some(tok) => return Error(UnexpectedToken(tok.clone())),
      None => return Error(UnexpectedEOF),
    }
  };
}

impl<'a> Parser<'a> {
  pub fn new(source: &'a Vec<u8>, tokens: &'a Vec<Token>) -> Parser<'a> {
    return Parser {
      source,
      tokens,
      index: 0,
      imports: Vec::new(),
      nodes: Vec::new(),
    };
  }

  pub fn parse_module(&mut self) -> Result<Node, ParseError> {
    while current_token_is!(self, Token::KeywordUse) {
      match self.parse_import() {
        Parsed(import) => self.imports.push(import),
        EOF => break,
        Error(err) => return Err(err),
      }
    }

    loop {
      match self.parse_expression() {
        Parsed(expr) => self.nodes.push(expr),
        EOF => break,
        Error(err) => return Err(err),
      }
    }

    Ok(Module {
      start: 0,
      end: self.tokens.last().map_or(0, |token| token.get_location().1),
      imports: self.imports.clone(),
      body: self.nodes.clone(),
    })
  }

  fn advance(&mut self) {
    self.index += 1;
  }

  fn current_token(&self) -> Option<&Token> {
    self.tokens.get(self.index)
  }

  fn current_token_location(&self) -> (usize, usize) {
    self
      .current_token()
      .expect("Must have a current token")
      .get_location()
  }

  fn next_token(&self) -> Option<&Token> {
    self.tokens.get(self.index + 1)
  }

  fn skip_line_breaks(&mut self) {
    while current_token_is!(self, Token::LineBreak) {
      self.advance()
    }
  }

  fn read_string(&self, start: usize, end: usize) -> String {
    let bytes = self.source[start..end].to_vec();
    String::from_utf8(bytes).expect("String is not UTF-8")
  }

  fn parse_qualified_identifier(&mut self) -> ParseResult {
    let first_id = match self.parse_lowercase_identifier() {
      Parsed(node) => node,
      other => return other,
    };

    let (first_start, _) = first_id.get_location();

    if current_token_is!(self, Token::Colon) {
      self.advance();

      match self.current_token() {
        Some(&Token::IdentifierLower(_, end)) => match self.parse_lowercase_identifier() {
          Parsed(id) => {
            return Parsed(QualifiedIdentifier {
              start: first_start,
              end,
              qualifier: Box::new(first_id),
              ident: Box::new(id),
              inferred_type: NodeType::Unknown,
            })
          }
          err => return err,
        },
        Some(&Token::IdentifierUpper(_, end)) => match self.parse_uppercase_identifier() {
          Parsed(id) => {
            return Parsed(QualifiedIdentifier {
              start: first_start,
              end: end,
              qualifier: Box::new(first_id),
              ident: Box::new(id),
              inferred_type: NodeType::Unknown,
            })
          }
          err => return err,
        },
        Some(tok) => return Error(UnexpectedToken(tok.clone())),
        None => return Error(UnexpectedEOF),
      };
    };

    Parsed(first_id)
  }

  fn parse_lowercase_identifier(&mut self) -> ParseResult {
    let (start, end, name) = expect_token_and_do!(self, Token::IdentifierLower, {
      let (start, end) = self.current_token_location();
      self.advance();
      (start, end, self.read_string(start, end))
    });

    Parsed(Identifier {
      start,
      end,
      name,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_uppercase_identifier(&mut self) -> ParseResult {
    let (start, end, name) = expect_token_and_do!(self, Token::IdentifierUpper, {
      let (start, end) = self.current_token_location();
      self.advance();
      (start, end, self.read_string(start, end))
    });

    Parsed(TypeIdentifier { start, end, name })
  }

  fn parse_string(&mut self) -> ParseResult {
    let (start, end, value) = match self.current_token() {
      Some(&Token::StringLiteral(start, end)) => (start, end, self.read_string(start, end)),
      _ => unreachable!(),
    };

    let node = StringLiteral {
      start,
      end,
      value,
      inferred_type: NodeType::Unknown,
    };

    self.advance();

    if current_token_is!(self, Token::InterpolationStart) {
      let mut parts = vec![node];
      let mut interpolation_end = end;

      while current_token_is!(self, Token::InterpolationStart) {
        self.advance();

        match self.parse_expression() {
          Parsed(part) => parts.push(part),
          other => return other,
        }

        expect_token_and_do!(self, Token::InterpolationEnd, {
          self.advance();
        });

        expect_token_and_do!(self, Token::StringLiteral, {
          let (start, end) = self.current_token_location();

          interpolation_end = end;

          parts.push(StringLiteral {
            start,
            end,
            value: self.read_string(start, end),
            inferred_type: NodeType::Unknown,
          });

          self.advance()
        })
      }

      return Parsed(StringInterpolation {
        start,
        end: interpolation_end,
        parts,
        inferred_type: NodeType::Unknown,
      });
    }

    Parsed(node)
  }

  fn parse_decimal_number(&mut self) -> ParseResult {
    let (start, end, value, raw_value) = expect_token_and_do!(self, Token::DecimalDigits, {
      let (start, end) = self.current_token_location();

      let string_value = self.read_string(start, end);
      let bytes = string_value.bytes().rev();

      let mut result: i64 = 0;
      let mut i: i64 = 1;

      for byte in bytes {
        let byte_value = match byte {
          b'0'..=b'9' => byte - 48,
          _ => unreachable!(),
        };

        result += (byte_value as i64) * i;
        i *= 10;
      }

      (start, end, NumericValue::Int(result), string_value)
    });

    self.advance();

    Parsed(NumericLiteral {
      start,
      end,
      value,
      raw_value,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_hex_number(&mut self) -> ParseResult {
    let (start, end, value, raw_value) = expect_token_and_do!(self, Token::HexDigits, {
      let (start, end) = self.current_token_location();

      let string_value = self.read_string(start, end);
      let bytes = string_value.bytes().rev();

      let mut result: i64 = 0;
      let mut i: i64 = 1;

      for byte in bytes {
        let byte_value = match byte {
          b'x' | b'X' => break,
          b'0'..=b'9' => byte - 48,
          b'a'..=b'f' => byte - 87,
          b'A'..=b'F' => byte - 55,
          _ => unreachable!(),
        };

        result += (byte_value as i64) * i;
        i *= 16;
      }

      (start, end, NumericValue::Int(result), string_value)
    });

    self.advance();

    Parsed(NumericLiteral {
      start,
      end,
      value,
      raw_value,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_octal_number(&mut self) -> ParseResult {
    let (start, end, value, raw_value) = expect_token_and_do!(self, Token::OctalDigits, {
      let (start, end) = self.current_token_location();

      let string_value = self.read_string(start, end);
      let bytes = string_value.bytes().rev();

      let mut result: i64 = 0;
      let mut i: i64 = 1;

      for byte in bytes {
        let byte_value = match byte {
          b'o' | b'O' => break,
          b'0'..=b'7' => byte - 48,
          _ => unreachable!(),
        };

        result += (byte_value as i64) * i;
        i *= 8;
      }

      (start, end, NumericValue::Int(result), string_value)
    });

    self.advance();

    Parsed(NumericLiteral {
      start,
      end,
      value,
      raw_value,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_binary_number(&mut self) -> ParseResult {
    let (start, end, value, raw_value) = expect_token_and_do!(self, Token::BinaryDigits, {
      let (start, end) = self.current_token_location();

      let string_value = self.read_string(start, end);
      let bytes = string_value.bytes().rev();

      let mut result: i64 = 0;
      let mut i: i64 = 1;

      for byte in bytes {
        let byte_value = match byte {
          b'b' | b'B' => break,
          b'0' => 0,
          b'1' => 1,
          _ => unreachable!(),
        };

        result += (byte_value as i64) * i;
        i *= 2;
      }

      (start, end, NumericValue::Int(result), string_value)
    });

    self.advance();

    Parsed(NumericLiteral {
      start,
      end,
      value,
      raw_value,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_parenthetical(&mut self) -> ParseResult {
    let paren_start = match self.current_token() {
      Some(&Token::LeftParen(start, _)) => start,
      _ => unreachable!(),
    };

    self.advance();

    let mut inner_exprs = Vec::new();

    while let Parsed(node) = self.parse_expression() {
      inner_exprs.push(node);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    self.skip_line_breaks();

    let paren_end = match self.current_token() {
      Some(&Token::RightParen(_, end)) => end,
      _ => return Error(UnclosedParentheses(self.index)),
    };

    self.advance();

    if inner_exprs.len() == 1 {
      return Parsed(Grouping {
        start: paren_start,
        end: paren_end,
        expr: Box::new(inner_exprs[0].clone()),
        inferred_type: NodeType::Unknown,
      });
    }

    Parsed(Tuple {
      start: paren_start,
      end: paren_end,
      entries: inner_exprs,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_block(&mut self) -> ParseResult {
    let block_start = match self.current_token() {
      Some(&Token::LeftBrace(start, _)) => start,
      _ => unreachable!(),
    };

    self.advance();

    let mut params = Vec::new();
    let mut body = Vec::new();

    while let Some(&Token::IdentifierLower(start, end)) = self.current_token() {
      if params.is_empty() {
        match self.next_token() {
          Some(&Token::Comma(..)) => {}
          Some(&Token::DoubleArrow(..)) => {}
          _ => break,
        }
      }

      let param = Identifier {
        start,
        end,
        name: self.read_string(start, end),
        inferred_type: NodeType::Unknown,
      };

      self.advance();

      params.push(param);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        Some(&Token::DoubleArrow(..)) => break,
        _ => return Error(MissingArrowAfterBlockParams(self.index)),
      }
    }

    if current_token_is!(self, Token::DoubleArrow) {
      self.advance();
    } else if !params.is_empty() {
      return Error(MissingArrowAfterBlockParams(self.index));
    }

    while let Parsed(node) = self.parse_expression() {
      body.push(node);
    }

    self.skip_line_breaks();

    let block_end = match self.current_token() {
      Some(&Token::RightBrace(_, end)) => end,
      _ => return Error(UnclosedBlock(self.index)),
    };

    self.advance();

    Parsed(Block {
      start: block_start,
      end: block_end,
      params,
      body,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_dict_entry_or_array_element(&mut self) -> Option<ParseResult> {
    if current_token_is!(self, Token::StringLiteral) {
      if let Parsed(string_node) = self.parse_string() {
        if current_token_is!(self, Token::Colon) {
          self.advance();

          match self.parse_expression() {
            Parsed(value_node) => {
              return Some(Parsed(DictEntry {
                start: 0,
                end: 0,
                key: Box::new(string_node),
                value: Box::new(value_node),
              }))
            }
            other => return Some(other),
          }
        } else {
          return Some(Parsed(string_node));
        }
      }
    } else {
      match self.parse_expression() {
        Parsed(element_node) => return Some(Parsed(element_node)),
        _ => {}
      }
    }

    None
  }

  fn parse_dict_or_array(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::LeftBracket(start, _)) => start,
      _ => unreachable!(),
    };

    self.advance();
    self.skip_line_breaks();

    let mut inner_exprs = Vec::new();
    let mut is_dict = None;

    match self.current_token() {
      Some(&Token::Colon(..)) => {
        self.advance();
        is_dict = Some(true);
      }
      _ => loop {
        match self.parse_dict_entry_or_array_element() {
          Some(Parsed(entry @ DictEntry { .. })) => {
            if let Some(false) = is_dict {
              return Error(UnexpectedDictEntryInArray(entry));
            } else {
              is_dict = Some(true);
            }

            inner_exprs.push(entry)
          }
          Some(Parsed(element)) => {
            if let Some(true) = is_dict {
              return Error(UnexpectedArrayElementInDict(element));
            } else {
              is_dict = Some(false);
            }

            inner_exprs.push(element)
          }
          Some(other) => return other,
          None => break,
        }

        self.skip_line_breaks();

        match self.current_token() {
          Some(&Token::Comma(..)) => {
            self.advance();
            self.skip_line_breaks();
          }
          _ => break,
        }
      },
    }

    self.skip_line_breaks();

    let end = match self.current_token() {
      Some(&Token::RightBracket(_, end)) => end,
      _ => return Error(UnclosedArray(self.index)),
    };

    self.advance();

    if let Some(true) = is_dict {
      return Parsed(Dict {
        start,
        end,
        entries: inner_exprs,
        inferred_type: NodeType::Unknown,
      });
    }

    return ParseResult::Parsed(Array {
      start,
      end,
      elements: inner_exprs,
      inferred_type: NodeType::Unknown,
    });
  }

  fn parse_any_calls_after_result(&mut self, previous: ParseResult) -> ParseResult {
    let mut current = previous.clone();
    let mut result = previous.clone();

    while let Parsed(node) = current {
      let (call_start, _) = node.get_location();

      if current_token_is!(self, Token::LeftParen) {
        match self.parse_parenthetical() {
          Parsed(Tuple {
            start,
            end,
            entries,
            ..
          }) => {
            current = Parsed(Call {
              start,
              end,
              callee: Box::new(node),
              arguments: entries,
              inferred_type: NodeType::Unknown,
            });

            result = current.clone();
            continue;
          }

          Parsed(expr_in_parens) => {
            let (_, expr_end) = expr_in_parens.get_location();

            current = Parsed(Call {
              start: call_start,
              end: expr_end,
              callee: Box::new(node),
              arguments: vec![ungroup(expr_in_parens)],
              inferred_type: NodeType::Unknown,
            });

            result = current.clone();
            continue;
          }

          other => return other,
        }
      } else if current_token_is!(self, Token::LeftBrace) {
        match self.parse_block() {
          Parsed(block) => {
            let (_, block_end) = block.get_location();

            current = Parsed(Call {
              start: call_start,
              end: block_end,
              callee: Box::new(node),
              arguments: vec![block],
              inferred_type: NodeType::Unknown,
            });

            result = current.clone();
            continue;
          }

          other => return other,
        }
      }

      break;
    }

    return result;
  }

  fn parse_chain(&mut self, previous: ParseResult) -> ParseResult {
    let previous_node = match previous {
      Parsed(node) => node,
      other => return other,
    };

    let chain_start = match self.current_token() {
      Some(&Token::Dot(start, _)) => start,
      _ => unreachable!(),
    };

    self.advance();

    let (end, ident) = match self.current_token() {
      Some(&Token::IdentifierLower(start, end)) => (
        end,
        Identifier {
          start,
          end,
          name: self.read_string(start, end),
          inferred_type: NodeType::Unknown,
        },
      ),
      Some(_) => return Error(UnexpectedTokenAfterDot(self.index)),
      None => return EOF,
    };

    self.advance();

    Parsed(Chain {
      start: chain_start,
      end,
      object: Box::new(previous_node),
      property: Box::new(ident),
    })
  }

  fn parse_match(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::KeywordMatch(start, _)) => {
        self.advance();
        start
      }
      _ => unreachable!(),
    };

    let matched_node = match self.parse_expression() {
      Parsed(node) => node,
      other => return other,
    };

    self.skip_line_breaks();

    let mut cases = Vec::new();
    let mut match_end = start;

    while let Some(&Token::Pipe(case_start, _)) = self.current_token() {
      self.advance();

      let case_pattern = match self.parse_expression() {
        Parsed(node) => node,
        other => return other,
      };

      match self.current_token() {
        Some(&Token::DoubleArrow(..)) => self.advance(),
        _ => return Error(MissingArrowInMatchCase(self.index)),
      };

      self.skip_line_breaks();

      let (case_end, case_body) = match self.parse_expression() {
        Parsed(node) => (node.get_location().1, node),
        other => return other,
      };

      self.skip_line_breaks();
      match_end = case_end;

      cases.push(MatchCase {
        start: case_start,
        end: case_end,
        pattern: Box::new(case_pattern),
        body: Box::new(case_body),
        inferred_type: NodeType::Unknown,
      });
    }

    if cases.is_empty() {
      return Error(MissingCasesInMatchExpression(self.index));
    }

    Parsed(Match {
      start,
      end: match_end,
      discriminant: Box::new(matched_node),
      cases,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_pattern(&mut self) -> ParseResult {
    match self.current_token() {
      Some(..) => self.parse_lowercase_identifier(),
      None => Error(ParseError::UnexpectedEOF),
    }
  }

  fn parse_reassignment(&mut self, previous: ParseResult) -> ParseResult {
    let (start, previous_node) = match previous {
      Parsed(node) => (node.get_location().0, node),
      other => return other,
    };

    match self.current_token() {
      Some(&Token::ColonEquals(..)) => self.advance(),
      _ => unreachable!(),
    }

    let (end, value_node) = match self.parse_expression() {
      Parsed(node) => (node.get_location().1, node),
      other => return other,
    };

    Parsed(Reassignment {
      start,
      end,
      left: Box::new(previous_node),
      right: Box::new(value_node),
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_assignment(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::KeywordLet(start, _)) => {
        self.advance();
        start
      }
      _ => unreachable!(),
    };

    let left = match self.parse_pattern() {
      Parsed(node) => Box::new(node),
      other => return other,
    };

    let is_constant = match self.current_token() {
      Some(&Token::Equals(..)) => true,
      Some(&Token::ColonEquals(..)) => false,
      Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      None => return Error(ParseError::UnexpectedEOF),
    };

    self.advance();
    self.skip_line_breaks();

    let (end, value_node) = match self.parse_expression() {
      Parsed(node) => (node.get_location().1, node),
      other => return other,
    };

    Parsed(Assignment {
      start,
      end,
      is_constant,
      left,
      right: Box::new(value_node),
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_method_definition(&mut self) -> ParseResult {
    let start = expect_token_and_do!(self, Token::KeywordDef, {
      let (token_start, _) = self.current_token_location();
      self.advance();
      token_start
    });

    let name = match self.parse_lowercase_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    expect_token_and_do!(self, Token::LeftParen, {
      self.advance();
    });

    let mut params = Vec::new();

    while current_token_is!(self, Token::IdentifierLower) {
      match self.parse_lowercase_identifier() {
        Parsed(node) => params.push(node),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      }

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(),
        Some(&Token::RightParen(..)) => break,
        Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
        None => return Error(ParseError::UnexpectedEOF),
      }
    }

    expect_token_and_do!(self, Token::RightParen, {
      self.advance();
    });

    expect_token_and_do!(self, Token::Equals, {
      self.advance();
    });

    let body = expect_token_and_do!(self, Token::LeftBrace, {
      match self.parse_block() {
        Parsed(node) => node,
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      }
    });

    let (_, end) = body.get_location();

    Parsed(MethodDefinition {
      start,
      end,
      name: Box::new(name),
      params,
      body: Box::new(body),
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_type_constraint(&mut self) -> ParseResult {
    let type_param = match self.parse_lowercase_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    expect_token_and_do!(self, Token::DoubleColon, {
      self.advance();
    });

    let type_value = match self.parse_uppercase_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    Parsed(TypeConstraint {
      start: 0,
      end: 0,
      type_param: Box::new(type_param),
      value: Box::new(type_value),
    })
  }

  fn parse_type_constraint_list(&mut self) -> ParseResult {
    let start = expect_token_and_do!(self, Token::KeywordWhere, {
      let (token_start, _) = self.current_token_location();
      self.advance();
      token_start
    });

    let mut constraints = Vec::new();

    match self.parse_type_constraint() {
      Parsed(node) => constraints.push(node),
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    while current_token_is!(self, Token::Comma) {
      self.advance();

      match self.parse_type_constraint() {
        Parsed(node) => constraints.push(node),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      };
    }

    let end = match constraints.last() {
      Some(constraint) => constraint.get_location().1,
      None => return Error(ParseError::MissingConstraintsAfterWhere(self.index)),
    };

    Parsed(TypeConstraintList {
      start,
      end,
      constraints,
    })
  }

  fn parse_type_expression(&mut self) -> ParseResult {
    let name = match self.parse_uppercase_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    Parsed(name)
  }

  fn parse_type_constructor_field(&mut self) -> ParseResult {
    if current_token_is!(self, Token::IdentifierLower) && next_token_is!(self, Token::DoubleColon) {
      let name = match self.parse_lowercase_identifier() {
        Parsed(node) => node,
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      };

      expect_token_and_do!(self, Token::DoubleColon, {
        self.advance();
      });

      let field_type = match self.parse_type_expression() {
        Parsed(node) => node,
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      };

      return Parsed(TypeConstructorField {
        start: 0,
        end: 0,
        name: Some(Box::new(name)),
        field_type: Box::new(field_type),
      });
    }

    let field_type = match self.parse_type_expression() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    return Parsed(TypeConstructorField {
      start: 0,
      end: 0,
      name: None,
      field_type: Box::new(field_type),
    });
  }

  fn parse_type_constructor_definition(&mut self) -> ParseResult {
    let name = match self.parse_uppercase_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    let mut fields = Vec::new();

    if current_token_is!(self, Token::LeftParen) {
      self.advance();

      match self.parse_type_constructor_field() {
        Parsed(node) => fields.push(node),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      };

      expect_token_and_do!(self, Token::RightParen, {
        self.advance();
      })
    }

    Parsed(TypeConstructorDefinition {
      start: 0,
      end: 0,
      name: Box::new(name),
      fields,
    })
  }

  fn parse_type_enum_definition(&mut self) -> ParseResult {
    let mut constructors = Vec::new();
    let mut start = None;

    self.skip_line_breaks();

    while current_token_is!(self, Token::Pipe) {
      if start.is_none() {
        start = Some(self.current_token_location().0);
      }

      expect_token_and_do!(self, Token::Pipe, {
        self.advance();
      });

      match self.parse_type_constructor_definition() {
        Parsed(node) => constructors.push(node),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      };

      self.skip_line_breaks();
    }

    Parsed(TypeEnumDefinition {
      start: start.unwrap(),
      end: 0,
      constructors,
    })
  }

  fn parse_type_definition(&mut self) -> ParseResult {
    let start = expect_token_and_do!(self, Token::KeywordType, {
      let (token_start, _) = self.current_token_location();
      self.advance();
      token_start
    });

    let name = match self.parse_uppercase_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    let mut type_params = Vec::new();

    if current_token_is!(self, Token::LeftParen) {
      expect_token_and_do!(self, Token::LeftParen, {
        self.advance();
      });

      while current_token_is!(self, Token::IdentifierLower) {
        match self.parse_lowercase_identifier() {
          Parsed(node) => type_params.push(node),
          EOF => return Error(ParseError::UnexpectedEOF),
          err => return err,
        }

        match self.current_token() {
          Some(&Token::Comma(..)) => self.advance(),
          Some(&Token::RightParen(..)) => break,
          Some(tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
          _ => return Error(ParseError::UnexpectedEOF),
        }
      }

      expect_token_and_do!(self, Token::RightParen, {
        self.advance();
      });
    }

    let constraint_list = if current_token_is!(self, Token::KeywordWhere) {
      match self.parse_type_constraint_list() {
        Parsed(list) => Some(Box::new(list)),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      }
    } else {
      None
    };

    self.skip_line_breaks();

    let parsed_value = match self.current_token() {
      Some(&Token::Pipe(..)) => self.parse_type_enum_definition(),
      // TODO: = for aliases, . for traits
      Some(tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      _ => return Error(ParseError::UnexpectedEOF),
    };

    let value = match parsed_value {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    let (_, end) = value.get_location();

    Parsed(TypeDefinition {
      start,
      end,
      name: Box::new(name),
      type_params,
      constraint_list,
      value: Box::new(value),
    })
  }

  fn parse_expression(&mut self) -> ParseResult {
    self.skip_line_breaks();

    let mut parsed = match self.current_token() {
      Some(&Token::KeywordDef(..)) => self.parse_method_definition(),
      Some(&Token::KeywordLet(..)) => self.parse_assignment(),
      Some(&Token::KeywordType(..)) => self.parse_type_definition(),
      Some(&Token::IdentifierLower(..)) => self.parse_qualified_identifier(),
      Some(&Token::IdentifierUpper(..)) => self.parse_uppercase_identifier(),
      Some(&Token::LeftParen(..)) => self.parse_parenthetical(),
      Some(&Token::StringLiteral(..)) => self.parse_string(),
      Some(&Token::KeywordMatch(..)) => self.parse_match(),
      Some(&Token::LeftBrace(..)) => self.parse_block(),
      Some(&Token::LeftBracket(..)) => self.parse_dict_or_array(),
      Some(&Token::DecimalDigits(..)) => self.parse_decimal_number(),
      Some(&Token::HexDigits(..)) => self.parse_hex_number(),
      Some(&Token::OctalDigits(..)) => self.parse_octal_number(),
      Some(&Token::BinaryDigits(..)) => self.parse_binary_number(),
      Some(&tok) => return Error(UnexpectedToken(tok.clone())),
      None => return EOF,
    };

    loop {
      match self.current_token() {
        Some(&Token::LeftParen(..)) | Some(&Token::LeftBrace(..)) => {
          parsed = self.parse_any_calls_after_result(parsed);
          continue;
        }
        Some(&Token::Dot(..)) | Some(&Token::ColonEquals(..)) => self.skip_line_breaks(),
        _ => break,
      }

      match self.current_token() {
        Some(&Token::Dot(..)) => parsed = self.parse_chain(parsed),
        Some(&Token::ColonEquals(..)) => parsed = self.parse_reassignment(parsed),
        _ => break,
      };
    }

    parsed
  }

  fn parse_import(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::KeywordUse(start, _)) => {
        self.advance();
        start
      }
      _ => unreachable!(),
    };

    let (path_end, path) = match self.current_token() {
      Some(&Token::ImportPath(start, end)) => (end, self.read_string(start, end)),
      Some(..) => return Error(UnexpectedTokenInImport(self.index)),
      None => return Error(UnexpectedEOF),
    };

    self.advance();

    let mut alias = None;
    let mut import_end = path_end;

    if current_token_is!(self, Token::KeywordAs) {
      self.advance();

      match self.current_token() {
        Some(&Token::IdentifierLower(start, end)) => {
          alias = Some(self.read_string(start, end));
          import_end = end;
        }
        _ => return Error(MissingAliasAfterAsInImport(self.index)),
      };

      self.advance();
    }

    self.skip_line_breaks();

    Parsed(Import {
      start,
      end: import_end,
      alias,
      module_name: path,
    })
  }
}
