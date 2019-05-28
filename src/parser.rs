use ast::{Node, NodeType};
use parser::ParseResult::{ParseError, Parsed};
use std::collections::HashMap;
use tokenizer::{Token, Tokenizer};

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
    let mut tokenizer = Tokenizer::new(source, preserve_comments);
    let mut tokens = Vec::new();
    let mut token_count = 0;
    while !tokenizer.is_done {
      tokens.push(tokenizer.read_token());
      token_count += 1;
    }

    println!("{:#?}", tokens);

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

  fn skip_skipped(&mut self) {
    loop {
      match self.next_token() {
        Some(Token::Skipped) => self.index += 1,
        _ => break,
      }
    }
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

  fn parse_number_literal(&mut self) -> Option<ParseResult> {
    None
  }

  fn parse_identifier(&mut self) -> Option<ParseResult> {
    let mut result = None;
    let mut to_advance = 0;

    if let Some(&Token::Identifier { line, value, .. }) = self.next_token() {
      to_advance = 1;

      result = Some(Parsed(Node::Identifier {
        line,
        value: to_string(value),
        inferred_type: NodeType::Unknown,
      }))
    }

    self.index += to_advance;
    result
  }

  pub fn parse_expression(&mut self) -> Option<ParseResult> {
    self.skip_skipped();

    if self.index >= self.token_count {
      return None;
    }

    let expr = self
      .parse_parenthetical()
      .or_else(|| self.parse_identifier())
      .or_else(|| self.parse_number_literal());

    expr
  }

  pub fn parse_module(&mut self) -> ParseResult {
    let mut body = Vec::new();

    loop {
      match self.parse_expression() {
        Some(Parsed(expr)) => body.push(expr),
        Some(ParseError(err)) => return ParseError(err),
        None => break,
      }
    }

    Parsed(Node::Module { body })
  }
}
