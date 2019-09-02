use crate::ast::{extract_location, Node, NodeType};
use crate::parser::ParseResult::{ParseError, Parsed};
use crate::tokens::Token;
use std::collections::HashMap;

pub struct Parser<'a> {
  tokens: &'a Vec<Token<'a>>,
  token_count: usize,
  index: usize,
}

#[derive(Debug)]
pub enum ParseResult {
  Parsed(Node),
  ParseError(String),
}

fn to_string(bytes: &[u8]) -> String {
  String::from_utf8(bytes.to_vec()).expect("String is not UTF-8")
}

impl<'a> Parser<'a> {
  pub fn from_tokens(tokens: &'a Vec<Token>) -> Parser<'a> {
    let token_count = tokens.len();

    return Parser {
      tokens,
      token_count,
      index: 0,
    };
  }

  fn next_token(&self) -> Option<&Token> {
    self.tokens.get(self.index)
  }

  fn next_next_token(&self) -> Option<&Token> {
    self.tokens.get(self.index + 1)
  }

  fn parse_comment(&mut self) -> Option<ParseResult> {
    let mut to_advance = 0;
    let mut node = None;

    if let Some(&Token::Comment { value, line, col }) = self.next_token() {
      to_advance = 1;

      node = Some(Parsed(Node::Comment {
        line,
        col_start: col,
        col_end: col + value.len(),
        value: to_string(value),
      }));
    }

    self.index += to_advance;
    node
  }

  fn parse_parenthetical(&mut self, body: &mut Vec<Node>) -> Option<ParseResult> {
    match self.next_token() {
      Some(&Token::LeftParen { .. }) => (),
      _ => return None,
    };

    self.index += 1;

    let mut expr = match self.parse_expression(body) {
      e @ Some(_) => e,
      None => Some(ParseError("Expected expr between ()".to_owned())),
    };

    match self.next_token() {
      Some(&Token::RightParen { .. }) => (),
      _ => expr = Some(ParseError("Missing )".to_owned())),
    };

    self.index += 1;
    expr
  }

  fn parse_block(&mut self) -> Option<ParseResult> {
    let (line_start, col_start) = match self.next_token() {
      Some(&Token::LeftBrace { line, col }) => (line, col),
      _ => return None,
    };

    self.index += 1;

    let mut params = Vec::new();

    match (self.next_token(), self.next_next_token()) {
      (Some(&Token::Identifier { .. }), Some(&Token::DoubleArrow { .. })) => {
        match self.parse_identifier() {
          Some(Parsed(id)) => params.push(id),
          err @ Some(ParseError(_)) => return err,
          None => (),
        }

        self.index += 1; // advance past the arrow
      }

      (Some(&Token::Identifier { .. }), Some(&Token::Comma { .. })) => loop {
        match self.parse_identifier() {
          Some(Parsed(id)) => params.push(id),
          err @ Some(ParseError(_)) => return err,
          None => return Some(ParseError("Expected identifier in block params".to_owned())),
        }

        match self.next_token() {
          Some(&Token::Comma { .. }) => self.index += 1,
          Some(&Token::DoubleArrow { .. }) => {
            self.index += 1;
            break;
          }
          _ => return Some(ParseError("Expected , or => after params".to_owned())),
        }
      },
      _ => (),
    }

    let mut body = Vec::new();

    loop {
      match self.parse_expression(&mut body) {
        Some(Parsed(expr)) => body.push(expr),
        err @ Some(ParseError(_)) => return err,
        None => break,
      }
    }

    let (line_end, col_end) = match self.next_token() {
      Some(&Token::RightBrace { line, col }) => (line, col),
      _ => return Some(ParseError("Missing }".to_owned())),
    };

    self.index += 1;

    Some(Parsed(Node::Block {
      line_start,
      col_start,
      line_end,
      col_end,
      params,
      body,
      inferred_type: NodeType::Unknown,
    }))
  }

  fn parse_number(&mut self) -> Option<ParseResult> {
    let mut result = None;
    let mut to_advance = 0;

    if let Some(&Token::Number { value, line, col }) = self.next_token() {
      to_advance = 1;

      result = Some(Parsed(Node::IntLiteral {
        line,
        col_start: col,
        col_end: col + value.len(),
        value: to_string(value),
        inferred_type: NodeType::Unknown,
      }))
    }

    self.index += to_advance;
    result
  }

  fn parse_string(&mut self, body: &mut Vec<Node>) -> Option<ParseResult> {
    let first_string_literal = match self.next_token() {
      Some(&Token::String {
        value,
        line_start,
        line_end,
        col_start,
        col_end,
      }) => Node::StringLiteral {
        line_start,
        line_end,
        col_start,
        col_end,
        value: to_string(value),
        inferred_type: NodeType::Unknown,
      },
      _ => return None,
    };

    self.index += 1;

    let mut interpolation_parts = Vec::new();

    while let Some(&Token::InterpolationStart { .. }) = self.next_token() {
      self.index += 1;

      match self.parse_expression(body) {
        Some(Parsed(expr)) => interpolation_parts.push(expr),
        _ => {
          return Some(ParseError(
            "Expected expression in interpolation".to_owned(),
          ))
        }
      };

      match self.next_token() {
        Some(&Token::InterpolationEnd { .. }) => self.index += 1,
        _ => return Some(ParseError("Expected ) to end interpolation".to_owned())),
      }

      match self.next_token() {
        Some(&Token::String {
          value,
          line_start,
          col_start,
          line_end,
          col_end,
        }) => interpolation_parts.push(Node::StringLiteral {
          line_start,
          line_end,
          col_start,
          col_end,
          value: to_string(value),
          inferred_type: NodeType::Unknown,
        }),
        _ => {
          return Some(ParseError(
            "Expected a string after interpolation".to_owned(),
          ))
        }
      }

      self.index += 1;
    }

    if interpolation_parts.is_empty() {
      return Some(Parsed(first_string_literal));
    }


    let (line_start, line_end, col_start, col_end) = extract_location(&first_string_literal);

    interpolation_parts.insert(0, first_string_literal);

    Some(Parsed(Node::StringInterpolation {
      line_start,
      line_end,
      col_start,
      col_end,
      parts: interpolation_parts,
      inferred_type: NodeType::Unknown,
    }))
  }

  fn parse_identifier(&mut self) -> Option<ParseResult> {
    let mut result = None;
    let mut to_advance = 0;

    if let Some(&Token::Identifier { value, line, col }) = self.next_token() {
      to_advance = 1;

      result = Some(Parsed(Node::Identifier {
        line,
        col_start: col,
        col_end: col + value.len(),
        name: to_string(value),
        inferred_type: NodeType::Unknown,
      }))
    }

    self.index += to_advance;
    result
  }

  fn parse_assignment(&mut self, body: &mut Vec<Node>) -> Option<ParseResult> {
    if body.is_empty() {
      return None;
    }

    let is_constant = match self.next_token() {
      Some(&Token::Equals { .. }) => true,
      Some(&Token::ColonEquals { .. }) => false,
      _ => return None,
    };

    self.index += 1;

    let left = body.pop().unwrap();

    let right = match self.parse_expression(body) {
      Some(Parsed(node)) => node,
      error @ Some(ParseError(_)) => return error,
      None => return Some(ParseError("Expected expression after =".to_owned())),
    };

    let (line_start, _, col_start, _) = extract_location(&left);
    let (_, line_end, _, col_end) = extract_location(&right);

    Some(Parsed(Node::Assignment {
      line_start,
      line_end,
      col_start,
      col_end,
      left: Box::new(left),
      right: Box::new(right),
      is_constant,
      inferred_type: NodeType::Unknown,
    }))
  }

  pub fn parse_expression(&mut self, body: &mut Vec<Node>) -> Option<ParseResult> {
    if self.index >= self.token_count {
      return None;
    }

    let expr = self
      .parse_parenthetical(body)
      .or_else(|| self.parse_block())
      .or_else(|| self.parse_identifier())
      .or_else(|| self.parse_assignment(body))
      .or_else(|| self.parse_string(body))
      .or_else(|| self.parse_number());

    expr
  }

  pub fn parse_module(&mut self) -> ParseResult {
    let mut body = Vec::new();
    let mut comments = HashMap::new();
    let mut done = false;

    loop {
      match self.parse_comment() {
        Some(Parsed(Node::Comment { line, col_start, col_end, value })) => {
          comments.insert(line, Node::Comment { line, col_start, col_end, value })
        },
        _ => None,
      };

      if done {
        break;
      }

      match self.parse_expression(&mut body) {
        Some(Parsed(expr)) => body.push(expr),
        Some(ParseError(err)) => return ParseError(err),
        None => done = true,
      }
    }

    Parsed(Node::Module {
      body,
      comments,
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
    assignment_constant,
    "x = 47"
  );

  assert_parsed_snapshot!(
    assignment_variable,
    "x := 47"
  );
}
