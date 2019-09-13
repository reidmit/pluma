use crate::ast::{get_node_location, Node, Node::*, NodeType, NumericValue, UnaryOperator};
use crate::tokens::{Token, get_token_location};
use crate::parser::{ParseError::*, ParseResult::*};

pub struct Parser<'a> {
  tokens: &'a Vec<Token<'a>>,
  token_count: usize,
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

#[derive(Debug, Clone)]
pub enum ParseError {
  UnexpectedToken(usize),
  UnexpectedEOF,
  UnclosedParentheses(usize),
  UnclosedBlock(usize),
  UnclosedArray(usize),
  UnclosedDict(usize),
  UnexpectedArrayElementInDict(Node),
  UnexpectedDictEntryInArray(Node),
  UnexpectedLineBreakInAssignment(usize),
  UnexpectedTokenAfterDot(usize),
  UnexpectedTokenInImport(usize),
  MissingArrowInMatchCase(usize),
  MissingArrowAfterBlockParams(usize),
}

fn to_string(bytes: &[u8]) -> String {
  String::from_utf8(bytes.to_vec()).expect("String is not UTF-8")
}

fn ungroup(node: Node) -> Node {
  match node {
    Grouping { expr, .. } => *expr,
    otherwise => otherwise,
  }
}

impl<'a> Parser<'a> {
  pub fn from_tokens(tokens: &'a Vec<Token>) -> Parser<'a> {
    let token_count = tokens.len();

    return Parser {
      tokens,
      token_count,
      index: 0,
      imports: Vec::new(),
      nodes: Vec::new(),
    };
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
    while let Some(&Token::LineBreak { .. }) = self.current_token() {
      self.advance(1)
    }
  }

  fn parse_identifier(&mut self) -> ParseResult {
    let (first_value, first_start) = match self.current_token() {
      Some(&Token::Identifier { value, start, .. }) => (value, start),
      _ => unreachable!()
    };

    let node = Identifier {
      start: first_start,
      end: first_start + first_value.len(),
      name: to_string(first_value),
      inferred_type: NodeType::Unknown,
    };

    self.advance(1);

    Parsed(node)
  }

  fn parse_string(&mut self) -> ParseResult {
    let (start, end, value) = match self.current_token() {
      Some(Token::StringLiteral { start, end, value }) => (*start, *end, value),
      _ => unreachable!(),
    };

    let node = StringLiteral {
      start,
      end,
      value: to_string(value),
      inferred_type: NodeType::Unknown,
    };

    self.advance(1);

    if let Some(&Token::InterpolationStart { .. }) = self.current_token() {
      let mut parts = vec![node];
      let mut interpolation_end = end;

      while let Some(&Token::InterpolationStart { .. }) = self.current_token() {
        self.advance(1);

        match self.parse_expression(true) {
          Parsed(part) => parts.push(part),
          other => return other,
        }

        match self.current_token() {
          Some(&Token::InterpolationEnd { .. }) => self.advance(1),
          Some(_) => return Error(UnexpectedToken(self.index)),
          None => return Error(UnexpectedEOF)
        }

        match self.current_token() {
          Some(&Token::StringLiteral { start, end, value }) => {
            interpolation_end = end;
            parts.push(StringLiteral {
              start,
              end,
              value: to_string(value),
              inferred_type: NodeType::Unknown,
            });
            self.advance(1)
          },
          Some(_) => return Error(UnexpectedToken(self.index)),
          None => return Error(UnexpectedEOF)
        }
      }

      return Parsed(StringInterpolation {
        start,
        end: interpolation_end,
        parts,
        inferred_type: NodeType::Unknown,
      })
    }

    Parsed(node)
  }

  fn parse_number(&mut self) -> ParseResult {
    let (start, end, value, raw_value) = match self.current_token() {
      Some(&Token::OctalDigits { start, end, value }) => {
        let string_value = to_string(&value);
        let bytes = string_value.bytes().rev();

        let mut result: i64 = 0;
        let mut i: i64 = 1;
        for byte in bytes {
          let byte_value = match byte {
            b'o' | b'O' => break,
            b'0'...b'7' => byte - 48,
            _ => unreachable!()
          };

          result += (byte_value as i64) * i;
          i *= 8;
        }

        (start, end, NumericValue::Int(result), value)
      },
      Some(&Token::HexDigits { start, end, value }) => {
        let string_value = to_string(&value);
        let bytes = string_value.bytes().rev();

        let mut result: i64 = 0;
        let mut i: i64 = 1;
        for byte in bytes {
          let byte_value = match byte {
            b'x' | b'X' => break,
            b'0'...b'9' => byte - 48,
            b'a'...b'f' => byte - 87,
            b'A'...b'F' => byte - 55,
            _ => unreachable!()
          };

          result += (byte_value as i64) * i;
          i *= 16;
        }

        (start, end, NumericValue::Int(result), value)
      },
      Some(&Token::DecimalDigits { start, end, value }) => {
        let string_value = to_string(&value);
        let bytes = string_value.bytes().rev();

        let mut result: i64 = 0;
        let mut i: i64 = 1;
        for byte in bytes {
          let byte_value = match byte {
            b'0'...b'9' => byte - 48,
            _ => unreachable!()
          };

          result += (byte_value as i64) * i;
          i *= 10;
        }

        (start, end, NumericValue::Int(result), value)
      },
      Some(&Token::BinaryDigits { start, end, value }) => {
        let string_value = to_string(&value);
        let bytes = string_value.bytes().rev();

        let mut result: i64 = 0;
        let mut i: i64 = 1;
        for byte in bytes {
          let byte_value = match byte {
            b'b' | b'B' => break,
            b'0' => 0,
            b'1' => 1,
            _ => unreachable!()
          };

          result += (byte_value as i64) * i;
          i *= 2;
        }

        (start, end, NumericValue::Int(result), value)
      },
      _ => unreachable!()
    };

    let node = Parsed(NumericLiteral {
      start,
      end,
      value,
      raw_value: to_string(raw_value),
      inferred_type: NodeType::Unknown,
    });

    self.advance(1);

    node
  }

  fn parse_parenthetical(&mut self) -> ParseResult {
    let paren_start = match self.current_token() {
      Some(&Token::LeftParen { start, .. }) => start,
      _ => unreachable!()
    };

    self.advance(1);

    let mut inner_exprs = Vec::new();

    while let Parsed(node) = self.parse_expression(true) {
      inner_exprs.push(node);

      match self.current_token() {
        Some(&Token::Comma { .. }) => self.advance(1),
        _ => break
      }
    }

    self.skip_line_breaks();

    let paren_end = match self.current_token() {
      Some(&Token::RightParen { end, .. }) => end,
      _ => return Error(UnclosedParentheses(self.index))
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

  fn parse_negated_expression(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::Minus { start, .. }) => start,
      _ => unreachable!()
    };

    self.advance(1);

    let (end, node) = match self.parse_expression(true) {
      Parsed(node) => (get_node_location(&node).1, node),
      other => return other
    };

    Parsed(UnaryOperation {
      start,
      end,
      operator: UnaryOperator::Minus,
      expr: Box::new(node),
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_block(&mut self) -> ParseResult {
    let block_start = match self.current_token() {
      Some(&Token::LeftBrace { start, .. }) => start,
      _ => unreachable!()
    };

    self.advance(1);

    let mut params = Vec::new();
    let mut body = Vec::new();

    while let Some(&Token::Identifier { start, end, value }) = self.current_token() {
      if params.is_empty() {
        match self.next_token() {
          Some(&Token::Comma { .. }) => {},
          Some(&Token::DoubleArrow { .. }) => {},
          _ => break,
        }
      }

      let param = Identifier {
        start,
        end,
        name: to_string(value),
        inferred_type: NodeType::Unknown,
      };

      self.advance(1);

      params.push(param);

      match self.current_token() {
        Some(&Token::Comma { .. }) => self.advance(1),
        Some(&Token::DoubleArrow { .. }) => break,
        _ => return Error(MissingArrowAfterBlockParams(self.index))
      }
    }

    match self.current_token() {
      Some(&Token::DoubleArrow { .. }) => self.advance(1),
      _ => {
        if !params.is_empty() {
          return Error(MissingArrowAfterBlockParams(self.index))
        }
      }
    }

    while let Parsed(node) = self.parse_expression(true) {
      body.push(node);
    }

    self.skip_line_breaks();

    let block_end = match self.current_token() {
      Some(&Token::RightBrace { end, .. }) => end,
      _ => return Error(UnclosedBlock(self.index))
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
    if let Some(&Token::StringLiteral { .. }) = self.current_token() {
      if let Parsed(string_node) = self.parse_string() {
        if let Some(&Token::Colon { .. }) = self.current_token() {
          self.advance(1);

          match self.parse_expression(true) {
            Parsed(value_node) => return Some(Parsed(DictEntry {
              start: 0,
              end: 0,
              key: Box::new(string_node),
              value: Box::new(value_node),
            })),
            other => return Some(other)
          }
        } else {
          return Some(Parsed(string_node));
        }
      }
    } else {
      match self.parse_expression(true) {
        Parsed(element_node) => return Some(Parsed(element_node)),
        _ => {},
      }
    }

    None
  }

  fn parse_dict_or_array(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::LeftBracket { start, .. }) => start,
      _ => unreachable!()
    };

    self.advance(1);
    self.skip_line_breaks();

    let mut inner_exprs = Vec::new();
    let mut is_dict = None;

    match self.current_token() {
      Some(&Token::Colon { .. }) => {
        self.advance(1);
        is_dict = Some(true);
      },
      _ => {
        loop {
          match self.parse_dict_entry_or_array_element() {
            Some(Parsed(entry @ DictEntry { .. })) => {
              if let Some(false) = is_dict {
                return Error(UnexpectedDictEntryInArray(entry))
              } else {
                is_dict = Some(true);
              }

              inner_exprs.push(entry)
            },
            Some(Parsed(element)) => {
              if let Some(true) = is_dict {
                return Error(UnexpectedArrayElementInDict(element))
              } else {
                is_dict = Some(false);
              }

              inner_exprs.push(element)
            },
            Some(other) => return other,
            None => break,
          }

          self.skip_line_breaks();

          match self.current_token() {
            Some(&Token::Comma { .. }) => {
              self.advance(1);
              self.skip_line_breaks();
            },
            _ => break
          }
        }
      }
    }

    self.skip_line_breaks();

    let end = match self.current_token() {
      Some(&Token::RightBracket { end, .. }) => end,
      _ => return Error(UnclosedArray(self.index))
    };

    self.advance(1);

    if let Some(true) = is_dict {
      return Parsed(Dict {
        start,
        end,
        entries: inner_exprs,
        inferred_type: NodeType::Unknown,
      })
    }

    Parsed(Array {
      start,
      end,
      elements: inner_exprs,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_any_calls_after_result(&mut self, previous: ParseResult) -> ParseResult {
    let mut current = previous.clone();
    let mut result = previous.clone();

    while let Parsed(node) = current {
      let (call_start, _) = get_node_location(&node);

      if let Some(&Token::LeftParen { .. }) = self.current_token() {
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
            continue
          },

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
            continue
          },

          other => return other
        }
      }

      break
    }

    return result;
  }

  fn parse_chain(&mut self, previous: ParseResult) -> ParseResult {
    let previous_node = match previous {
      Parsed(node) => node,
      other => return other,
    };

    let chain_start = match self.current_token() {
      Some(&Token::Dot { start, .. }) => start,
      _ => unreachable!()
    };

    self.advance(1);

    let (end, ident) = match self.current_token() {
      Some(&Token::Identifier { start, end, value }) => (end, Identifier {
        start,
        end,
        name: to_string(value),
        inferred_type: NodeType::Unknown,
      }),
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

  fn parse_match(&mut self, previous: ParseResult) -> ParseResult {
    let (start, previous_node) = match previous {
      Parsed(node) => (get_node_location(&node).0, node),
      other => return other,
    };

    let mut cases = Vec::new();
    let mut match_end = start;

    while let Some(&Token::Pipe { start: case_start, .. }) = self.current_token() {
      self.advance(1);

      let case_pattern = match self.parse_expression(false) {
        Parsed(node) => node,
        other => return other,
      };

      match self.current_token() {
        Some(&Token::DoubleArrow { .. }) => self.advance(1),
        _ => return Error(MissingArrowInMatchCase(self.index))
      };

      self.skip_line_breaks();

      let (case_end, case_body) = match self.parse_expression(false) {
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

    Parsed(Match {
      start,
      end: match_end,
      discriminant: Box::new(previous_node),
      cases,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_assignment(&mut self, previous: ParseResult) -> ParseResult {
    let (start, previous_node) = match previous {
      Parsed(node) => (get_node_location(&node).0, node),
      other => return other,
    };

    let is_constant = match self.current_token() {
      Some(&Token::Equals { .. }) => true,
      Some(&Token::ColonEquals { .. }) => false,
      _ => unreachable!()
    };

    self.advance(1);

    if let Some(&Token::LineBreak { .. }) = self.current_token() {
      return Error(UnexpectedLineBreakInAssignment(self.index));
    };

    let (end, value_node) = match self.parse_expression(true) {
      Parsed(node) => (get_node_location(&node).1, node),
      other => return other,
    };

    Parsed(Assignment {
      start,
      end,
      is_constant,
      left: Box::new(previous_node),
      right: Box::new(value_node),
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_expression(&mut self, allow_case: bool) -> ParseResult {
    self.skip_line_breaks();

    let mut parsed = match self.current_token() {
      Some(&Token::Minus { .. }) => self.parse_negated_expression(),
      Some(&Token::Identifier { .. }) => self.parse_identifier(),
      Some(&Token::LeftParen { .. }) => self.parse_parenthetical(),
      Some(&Token::StringLiteral { .. }) => self.parse_string(),
      Some(&Token::LeftBrace { .. }) => self.parse_block(),
      Some(&Token::LeftBracket { .. }) => self.parse_dict_or_array(),
      Some(&Token::OctalDigits { .. })
        | Some(&Token::HexDigits { .. })
        | Some(&Token::DecimalDigits { .. })
        | Some(&Token::BinaryDigits { .. }) => self.parse_number(),
      Some(_) => Error(UnexpectedToken(self.index)),
      None => EOF,
    };

    let mut parsed_call = false;

    loop {
      match self.current_token() {
        Some(&Token::LeftParen { .. }) => {
          parsed = self.parse_any_calls_after_result(parsed);
          parsed_call = true;
        },
        _ => break,
      };
    }

    if parsed_call {
      return parsed;
    }

    loop {
      self.skip_line_breaks();

      match self.current_token() {
        Some(&Token::Dot { .. }) => parsed = self.parse_chain(parsed),
        Some(&Token::Pipe { .. }) if allow_case => parsed = self.parse_match(parsed),
        Some(&Token::Equals { .. })
          | Some(&Token::ColonEquals { .. }) => parsed = self.parse_assignment(parsed),
        _ => break,
      };
    }

    parsed
  }

  fn parse_import(&mut self) -> ParseResult {
    let start = match self.current_token() {
      Some(&Token::Plus { start, .. }) => {
        self.advance(1);
        start
      },
      _ => unreachable!(),
    };

    let alias = match self.current_token() {
      Some(&Token::Identifier { value, .. }) => Some(to_string(value)),
      _ => None
    };

    if alias.is_some() {
      self.advance(1);
    }

    let (end, path) = match self.current_token() {
      Some(&Token::StringLiteral { end, value, .. }) => (end, to_string(value)),
      Some(..) => return Error(UnexpectedTokenInImport(self.index)),
      None => return Error(UnexpectedEOF),
    };

    self.advance(1);
    self.skip_line_breaks();

    Parsed(Import {
      start,
      end,
      alias,
      path,
    })
  }

  pub fn parse_module(&mut self) -> Result<Node, ParseError> {
    while let Some(&Token::Plus { .. }) = self.current_token() {
      match self.parse_import() {
        Parsed(import) => self.imports.push(import),
        EOF => break,
        Error(err) => return Err(err),
      }
    }

    loop {
      match self.parse_expression(true) {
        Parsed(expr) => self.nodes.push(expr),
        EOF => break,
        Error(err) => return Err(err),
      }
    }

    Ok(Module {
      start: 0,
      end: self.tokens.last().map_or(0, |token| get_token_location(token).1),
      imports: self.imports.clone(),
      body: self.nodes.clone(),
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::tokenizer::Tokenizer;
  use crate::assert_parsed_snapshot;
  use insta::assert_snapshot;

  assert_parsed_snapshot!(
    empty,
    ""
  );

  assert_parsed_snapshot!(
    identifier,
    "hello"
  );

  assert_parsed_snapshot!(
    number,
    "47"
  );

  assert_parsed_snapshot!(
    string,
    "\"hello\""
  );

  assert_parsed_snapshot!(
    string_with_interpolation,
    "\"hello $(name)!\""
  );

  assert_parsed_snapshot!(
    string_with_nested_interpolation,
    "\"hello $(\"aa $(name) bb\")!\""
  );

  assert_parsed_snapshot!(
    assignment_constant,
    "x = 47"
  );

  assert_parsed_snapshot!(
    assignment_variable,
    "x := 47"
  );
}