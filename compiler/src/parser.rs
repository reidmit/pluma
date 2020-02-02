use crate::ast::{get_node_location, Node, Node::*, NodeType, NumericValue};
use crate::errors::ParseError;
use crate::parser::{ParseError::*, ParseResult::*};
use crate::tokens::{get_token_location, Token};

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
    while let Some(&Token::KeywordUse(..)) = self.current_token() {
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
      end: self
        .tokens
        .last()
        .map_or(0, |token| get_token_location(token).1),
      imports: self.imports.clone(),
      body: self.nodes.clone(),
    })
  }

  fn advance(&mut self, amount: usize) {
    self.index += amount;
  }

  fn current_token(&self) -> Option<&Token> {
    self.tokens.get(self.index)
  }

  fn next_token(&self) -> Option<&Token> {
    self.tokens.get(self.index + 1)
  }

  fn skip_line_breaks(&mut self) {
    while let Some(&Token::LineBreak(..)) = self.current_token() {
      self.advance(1)
    }
  }

  fn read_string(&self, start: usize, end: usize) -> String {
    let bytes = self.source[start..end].to_vec();
    String::from_utf8(bytes).expect("String is not UTF-8")
  }

  fn parse_identifier(&mut self) -> ParseResult {
    let mut qualifier = None;

    let (start, mut end, mut name) = match self.current_token() {
      Some(&Token::Identifier(start, end)) => (start, end, self.read_string(start, end)),
      Some(tok) => return Error(UnexpectedToken(tok.clone())),
      None => return Error(UnexpectedEOF),
    };

    self.advance(1);

    if let Some(&Token::Colon(..)) = self.current_token() {
      self.advance(1);

      match self.current_token() {
        Some(&Token::Identifier(part_start, part_end)) => {
          qualifier = Some(Box::new(Node::Identifier {
            name,
            start,
            end,
            qualifier: None,
            inferred_type: NodeType::Unknown,
          }));
          name = self.read_string(part_start, part_end);
          end = part_end;

          self.advance(1);
        }
        Some(tok) => return Error(UnexpectedToken(tok.clone())),
        None => return Error(UnexpectedEOF),
      };
    };

    Parsed(Identifier {
      start,
      end,
      name,
      qualifier,
      inferred_type: NodeType::Unknown,
    })
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

    self.advance(1);

    if let Some(&Token::InterpolationStart(..)) = self.current_token() {
      let mut parts = vec![node];
      let mut interpolation_end = end;

      while let Some(&Token::InterpolationStart(..)) = self.current_token() {
        self.advance(1);

        match self.parse_expression() {
          Parsed(part) => parts.push(part),
          other => return other,
        }

        match self.current_token() {
          Some(&Token::InterpolationEnd(..)) => self.advance(1),
          Some(tok) => return Error(UnexpectedToken(tok.clone())),
          None => return Error(UnexpectedEOF),
        }

        match self.current_token() {
          Some(&Token::StringLiteral(start, end)) => {
            interpolation_end = end;
            parts.push(StringLiteral {
              start,
              end,
              value: self.read_string(start, end),
              inferred_type: NodeType::Unknown,
            });
            self.advance(1)
          }
          Some(tok) => return Error(UnexpectedToken(tok.clone())),
          None => return Error(UnexpectedEOF),
        }
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

  fn parse_number(&mut self) -> ParseResult {
    let (start, end, value, raw_value) = match self.current_token() {
      Some(&Token::OctalDigits(start, end)) => {
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
      }
      Some(&Token::HexDigits(start, end)) => {
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
      }
      Some(&Token::DecimalDigits(start, end)) => {
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
      }
      Some(&Token::BinaryDigits(start, end)) => {
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
      }
      _ => unreachable!(),
    };

    self.advance(1);

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

    self.advance(1);

    let mut inner_exprs = Vec::new();

    while let Parsed(node) = self.parse_expression() {
      inner_exprs.push(node);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(1),
        _ => break,
      }
    }

    self.skip_line_breaks();

    let paren_end = match self.current_token() {
      Some(&Token::RightParen(_, end)) => end,
      _ => return Error(UnclosedParentheses(self.index)),
    };

    self.advance(1);

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

    self.advance(1);

    let mut params = Vec::new();
    let mut body = Vec::new();

    while let Some(&Token::Identifier(start, end)) = self.current_token() {
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
        qualifier: None,
        inferred_type: NodeType::Unknown,
      };

      self.advance(1);

      params.push(param);

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(1),
        Some(&Token::DoubleArrow(..)) => break,
        _ => return Error(MissingArrowAfterBlockParams(self.index)),
      }
    }

    match self.current_token() {
      Some(&Token::DoubleArrow(..)) => self.advance(1),
      _ => {
        if !params.is_empty() {
          return Error(MissingArrowAfterBlockParams(self.index));
        }
      }
    }

    while let Parsed(node) = self.parse_expression() {
      body.push(node);
    }

    self.skip_line_breaks();

    let block_end = match self.current_token() {
      Some(&Token::RightBrace(_, end)) => end,
      _ => return Error(UnclosedBlock(self.index)),
    };

    self.advance(1);

    Parsed(Block {
      start: block_start,
      end: block_end,
      params,
      body,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_dict_entry_or_array_element(&mut self) -> Option<ParseResult> {
    if let Some(&Token::StringLiteral(..)) = self.current_token() {
      if let Parsed(string_node) = self.parse_string() {
        if let Some(&Token::Colon(..)) = self.current_token() {
          self.advance(1);

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

    self.advance(1);
    self.skip_line_breaks();

    let mut inner_exprs = Vec::new();
    let mut is_dict = None;

    match self.current_token() {
      Some(&Token::Colon(..)) => {
        self.advance(1);
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
            self.advance(1);
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

    self.advance(1);

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
      let (call_start, _) = get_node_location(&node);

      if let Some(&Token::LeftParen(..)) = self.current_token() {
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
            let (_, expr_end) = get_node_location(&expr_in_parens);

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
      } else if let Some(&Token::LeftBrace(..)) = self.current_token() {
        match self.parse_block() {
          Parsed(block) => {
            let (_, block_end) = get_node_location(&block);

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

    self.advance(1);

    let (end, ident) = match self.current_token() {
      Some(&Token::Identifier(start, end)) => (
        end,
        Identifier {
          start,
          end,
          name: self.read_string(start, end),
          qualifier: None,
          inferred_type: NodeType::Unknown,
        },
      ),
      Some(_) => return Error(UnexpectedTokenAfterDot(self.index)),
      None => return EOF,
    };

    self.advance(1);

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
        self.advance(1);
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
      self.advance(1);

      let case_pattern = match self.parse_expression() {
        Parsed(node) => node,
        other => return other,
      };

      match self.current_token() {
        Some(&Token::DoubleArrow(..)) => self.advance(1),
        _ => return Error(MissingArrowInMatchCase(self.index)),
      };

      self.skip_line_breaks();

      let (case_end, case_body) = match self.parse_expression() {
        Parsed(node) => (get_node_location(&node).1, node),
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
      Some(..) => self.parse_identifier(),
      None => Error(ParseError::UnexpectedEOF),
    }
  }

  fn parse_reassignment(&mut self, previous: ParseResult) -> ParseResult {
    let (start, previous_node) = match previous {
      Parsed(node) => (get_node_location(&node).0, node),
      other => return other,
    };

    match self.current_token() {
      Some(&Token::ColonEquals(..)) => self.advance(1),
      _ => unreachable!(),
    }

    let (end, value_node) = match self.parse_expression() {
      Parsed(node) => (get_node_location(&node).1, node),
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
        self.advance(1);
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

    self.advance(1);
    self.skip_line_breaks();

    let (end, value_node) = match self.parse_expression() {
      Parsed(node) => (get_node_location(&node).1, node),
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
    let start = match self.current_token() {
      Some(&Token::KeywordDef(start, _)) => {
        self.advance(1);
        start
      }
      _ => unreachable!(),
    };

    let name = match self.parse_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    match self.current_token() {
      Some(&Token::LeftParen(..)) => self.advance(1),
      Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      None => return Error(ParseError::UnexpectedEOF),
    };

    let mut params = Vec::new();

    while let Some(&Token::Identifier(..)) = self.current_token() {
      match self.parse_identifier() {
        Parsed(node) => params.push(node),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      }

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(1),
        Some(&Token::RightParen(..)) => break,
        Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
        None => return Error(ParseError::UnexpectedEOF),
      }
    }

    match self.current_token() {
      Some(&Token::RightParen(..)) => self.advance(1),
      Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      None => return Error(ParseError::UnexpectedEOF),
    };

    match self.current_token() {
      Some(&Token::Equals(..)) => self.advance(1),
      Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      None => return Error(ParseError::UnexpectedEOF),
    };

    let body = match self.current_token() {
      Some(&Token::LeftBrace(..)) => match self.parse_block() {
        Parsed(node) => node,
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      },
      Some(&tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      None => return Error(ParseError::UnexpectedEOF),
    };

    let (_, end) = get_node_location(&body);

    Parsed(MethodDefinition {
      start,
      end,
      name: Box::new(name),
      params,
      body: Box::new(body),
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_type_definition(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::KeywordType(start, _)) => {
        self.advance(1);
        start
      }
      _ => unreachable!(),
    };

    let name = match self.parse_identifier() {
      Parsed(node) => node,
      EOF => return Error(ParseError::UnexpectedEOF),
      err => return err,
    };

    match self.current_token() {
      Some(&Token::LeftParen(..)) => self.advance(1),
      Some(tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      _ => return Error(ParseError::UnexpectedEOF),
    };

    let mut params = Vec::new();

    while let Some(&Token::Identifier(..)) = self.current_token() {
      match self.parse_identifier() {
        Parsed(node) => params.push(node),
        EOF => return Error(ParseError::UnexpectedEOF),
        err => return err,
      }

      match self.current_token() {
        Some(&Token::Comma(..)) => self.advance(1),
        Some(&Token::RightParen(..)) => break,
        Some(tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
        _ => return Error(ParseError::UnexpectedEOF),
      }
    }

    match self.current_token() {
      Some(&Token::RightParen(..)) => self.advance(1),
      Some(tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      _ => return Error(ParseError::UnexpectedEOF),
    };

    match self.current_token() {
      Some(&Token::Equals(..)) => self.advance(1),
      Some(tok) => return Error(ParseError::UnexpectedToken(tok.clone())),
      _ => return Error(ParseError::UnexpectedEOF),
    };

    Parsed(TypeDefinition {
      start,
      end: 0,
      name: Box::new(name),
    })
  }

  fn parse_expression(&mut self) -> ParseResult {
    self.skip_line_breaks();

    let mut parsed = match self.current_token() {
      Some(&Token::KeywordDef(..)) => self.parse_method_definition(),
      Some(&Token::KeywordLet(..)) => self.parse_assignment(),
      Some(&Token::KeywordType(..)) => self.parse_type_definition(),
      Some(&Token::Identifier(..)) => self.parse_identifier(),
      Some(&Token::LeftParen(..)) => self.parse_parenthetical(),
      Some(&Token::StringLiteral(..)) => self.parse_string(),
      Some(&Token::KeywordMatch(..)) => self.parse_match(),
      Some(&Token::LeftBrace(..)) => self.parse_block(),
      Some(&Token::LeftBracket(..)) => self.parse_dict_or_array(),
      Some(&Token::OctalDigits(..))
      | Some(&Token::HexDigits(..))
      | Some(&Token::DecimalDigits(..))
      | Some(&Token::BinaryDigits(..)) => self.parse_number(),
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
        self.advance(1);
        start
      }
      _ => unreachable!(),
    };

    let (path_end, path) = match self.current_token() {
      Some(&Token::ImportPath(start, end)) => (end, self.read_string(start, end)),
      Some(..) => return Error(UnexpectedTokenInImport(self.index)),
      None => return Error(UnexpectedEOF),
    };

    self.advance(1);

    let mut alias = None;
    let mut import_end = path_end;

    if let Some(&Token::KeywordAs(..)) = self.current_token() {
      self.advance(1);

      match self.current_token() {
        Some(&Token::Identifier(start, end)) => {
          alias = Some(self.read_string(start, end));
          import_end = end;
        }
        _ => return Error(MissingAliasAfterAsInImport(self.index)),
      };

      self.advance(1);
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
