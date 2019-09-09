use crate::ast::{get_node_location, Node, NodeType, NumericValue};
use crate::tokens::{Token, get_token_location};
use crate::parser::ParseError::*;

pub struct Parser<'a> {
  tokens: &'a Vec<Token<'a>>,
  token_count: usize,
  index: usize,
  nodes: Vec<Node>,
}

#[derive(Debug, Clone)]
enum ParseResult {
  Ok(Node),
  EOF,
  Error(ParseError),
}

#[derive(Debug, Clone)]
pub enum ParseError {
  UnexpectedToken(String, usize),
  UnexpectedEOF(String),
  UnclosedParentheses(usize),
}

fn to_string(bytes: &[u8]) -> String {
  String::from_utf8(bytes.to_vec()).expect("String is not UTF-8")
}

fn ungroup(node: Node) -> Node {
  match node {
    Node::Grouping { expr, .. } => *expr,
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
      nodes: Vec::new(),
    };
  }

  fn advance(&mut self, amount: usize) {
    self.index += amount;
  }

  fn current_token(&self) -> Option<&Token> {
    self.tokens.get(self.index)
  }

  fn skip_line_breaks(&mut self) {
    while let Some(&Token::LineBreak { .. }) = self.current_token() {
      self.advance(1)
    }
  }

  fn parse_identifier(&mut self) -> ParseResult {
    // Make sure we're starting with an identifier token, and grab its value & index
    let (first_value, first_start) = match self.current_token() {
      Some(&Token::Identifier { value, start, .. }) => (value, start),
      _ => unreachable!()
    };

    // Make a node for the identifier & advance past it
    let mut node = Node::Identifier {
      start: first_start,
      end: first_start + first_value.len(),
      name: to_string(first_value),
      inferred_type: NodeType::Unknown,
    };

    self.advance(1);

    // While we see a '.', parse any chained identifiers
    while let Some(&Token::Dot { .. }) = self.current_token() {
      self.advance(1);

      let (value, id_start, id_end) = match self.current_token() {
        Some(&Token::Identifier { value, start, end }) => (value, start, end),
        Some(_) => return ParseResult::Error(UnexpectedToken(
          "Unexpected token after '.'. Expected to see an identifier.".to_owned(),
          self.index,
        )),
        None => return ParseResult::Error(UnexpectedEOF(
          "Unexpected end-of-file after '.'. Expected to see an identifier.".to_owned(),
        )),
      };

      let (node_start, _) = get_node_location(&node);

      let child_node = Node::Identifier {
        start: id_start,
        end: id_end,
        name: to_string(value),
        inferred_type: NodeType::Unknown,
      };

      self.advance(1);

      node = Node::Chain {
        start: node_start,
        end: id_end,
        object: Box::new(node),
        property: Box::new(child_node),
      }
    }

    ParseResult::Ok(node)
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

    let node = ParseResult::Ok(Node::NumericLiteral {
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

    while let ParseResult::Ok(node) = self.parse_expression() {
      inner_exprs.push(node);

      match self.current_token() {
        Some(&Token::Comma { .. }) => self.advance(1),
        _ => break
      }
    }

    self.skip_line_breaks();

    let paren_end = match self.current_token() {
      Some(&Token::RightParen { end, .. }) => end,
      _ => return ParseResult::Error(UnclosedParentheses(self.index))
    };

    self.advance(1);

    if inner_exprs.len() == 1 {
      return ParseResult::Ok(Node::Grouping {
        start: paren_start,
        end: paren_end,
        expr: Box::new(inner_exprs[0].clone()),
        inferred_type: NodeType::Unknown,
      });
    }

    ParseResult::Ok(Node::Tuple {
      start: paren_start,
      end: paren_end,
      entries: inner_exprs,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_any_calls_after_result(&mut self, previous: ParseResult) -> ParseResult {
    let mut current = previous.clone();
    let mut result = previous.clone();

    while let ParseResult::Ok(node) = current {
      let (call_start, _) = get_node_location(&node);

      if let Some(&Token::LeftParen { .. }) = self.current_token() {
        match self.parse_parenthetical() {
          ParseResult::Ok(Node::Tuple {
            start,
            end,
            entries,
            ..
          }) => {
            current = ParseResult::Ok(Node::Call {
              start,
              end,
              callee: Box::new(node),
              arguments: entries,
              inferred_type: NodeType::Unknown,
            });

            result = current.clone();
            continue
          },

          ParseResult::Ok(expr_in_parens) => {
            let (_, expr_end) = get_node_location(&expr_in_parens);

            current = ParseResult::Ok(Node::Call {
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

  fn parse_expression(&mut self) -> ParseResult {
    self.skip_line_breaks();

    let parsed = match self.current_token() {
      Some(&Token::Identifier { .. }) => self.parse_identifier(),
      Some(&Token::LeftParen { .. }) => self.parse_parenthetical(),
      Some(&Token::OctalDigits { .. })
        | Some(&Token::HexDigits { .. })
        | Some(&Token::DecimalDigits { .. })
        | Some(&Token::BinaryDigits { .. }) => self.parse_number(),
      Some(_) => ParseResult::Error(
        UnexpectedToken("Unexpected token".to_owned(), self.index)
      ),
      None => ParseResult::EOF,
    };

    self.parse_any_calls_after_result(parsed)
  }

  pub fn parse_module(&mut self) -> Result<Node, ParseError> {
    loop {
      match self.parse_expression() {
        ParseResult::Ok(expr) => self.nodes.push(expr),
        ParseResult::EOF => break,
        ParseResult::Error(err) => return Err(err),
      }
    }

    Ok(Node::Module {
      start: 0,
      end: self.tokens.last().map_or(0, |token| get_token_location(token).1),
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
