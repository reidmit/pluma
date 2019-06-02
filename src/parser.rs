use crate::ast::{extract_location,Node, NodeType};
use crate::parser::ParseResult::{ParseError, Parsed};
use crate::tokenizer::Tokenizer;
use crate::tokens::{ Token};
use std::collections::HashMap;

pub struct Parser<'a> {
  source: &'a Vec<u8>,
  tokens: Vec<Token<'a>>,
  token_count: usize,
  index: usize,
  comments: HashMap<usize, Node>,
  body: Vec<Node>,
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
  pub fn from_source(source: &'a Vec<u8>, preserve_comments: bool) -> Parser<'a> {
    let tokens = Tokenizer::from_source(source).collect_tokens();
    let token_count = tokens.len();

    // println!("{:#?}", tokens);

    return Parser {
      source,
      tokens,
      token_count,
      index: 0,
      comments: HashMap::new(),
      body: Vec::new(),
    };
  }

  fn next_token(&self) -> Option<&Token> {
    self.tokens.get(self.index)
  }

  fn parse_parenthetical(&mut self) -> Option<ParseResult> {
    match self.next_token() {
      Some(&Token::LeftParen { .. }) => (),
      _ => return None,
    };

    self.index += 1;

    let mut expr = match self.parse_expression() {
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

  fn parse_string(&mut self) -> Option<ParseResult> {
    let first_string_literal = match self.next_token() {
      Some(&Token::String { value, line_start, col_start, line_end, col_end }) => Node::StringLiteral {
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

      match self.parse_expression() {
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
        Some(&Token::String { value, line_start, col_start, line_end, col_end }) => {
          interpolation_parts.push(Node::StringLiteral {
            line_start,
            line_end,
            col_start,
            col_end,
            value: to_string(value),
            inferred_type: NodeType::Unknown,
          })
        }
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

  fn parse_assignment(&mut self) -> Option<ParseResult> {
    if self.body.is_empty() {
      return None;
    }

    let left = self.body.pop().unwrap();
    let mut is_constant = true;

    match self.next_token() {
      Some(&Token::Equals { .. }) => (),
      Some(&Token::ColonEquals { .. }) => is_constant = false,
      _ => return None,
    }

    self.index += 1;

    let right = match self.parse_expression() {
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

  pub fn parse_expression(&mut self) -> Option<ParseResult> {
    if self.index >= self.token_count {
      return None;
    }

    let expr = self
      .parse_parenthetical()
      .or_else(|| self.parse_identifier())
      .or_else(|| self.parse_assignment())
      .or_else(|| self.parse_string())
      .or_else(|| self.parse_number());

    expr
  }

  pub fn parse_module(&mut self) -> ParseResult {
    loop {
      match self.parse_expression() {
        Some(Parsed(expr)) => self.body.push(expr),
        Some(ParseError(err)) => return ParseError(err),
        None => break,
      }
    }

    Parsed(Node::Module {
      body: self.body.clone(),
    })
  }
}
