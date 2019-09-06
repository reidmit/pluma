use crate::ast::{extract_location, Node, NodeType};
use crate::tokens::Token;
use crate::parser::ParseError::*;

pub struct Parser<'a> {
  tokens: &'a Vec<Token<'a>>,
  token_count: usize,
  index: usize,
  nodes: Vec<Node>,
}

#[derive(Debug)]
pub enum SourceLocation {
  Char { line: usize, col: usize },
  CharSpan { line: usize, col_start: usize, col_end: usize },
  LineSpan { line_start: usize, line_end: usize, col_start: usize, col_end: usize },
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

  fn parse_identifier(&mut self) -> ParseResult {
    let (first_value, first_line, first_col) = match self.current_token() {
      Some(&Token::Identifier { value, line, col }) => (value, line, col),
      _ => unreachable!()
    };

    let mut node = Node::Identifier {
      line: first_line,
      col_start: first_col,
      col_end: first_col + first_value.len(),
      name: to_string(first_value),
      inferred_type: NodeType::Unknown,
    };

    self.advance(1);

    while let Some(&Token::Dot { .. }) = self.current_token() {
      self.advance(1);

      let (value, line, col) = match self.current_token() {
        Some(&Token::Identifier { value, line, col }) => (value, line, col),
        Some(_) => return ParseResult::Error(UnexpectedToken(
          "Unexpected token after '.'. Expected to see an identifier.".to_owned(),
          self.index,
        )),
        None => return ParseResult::Error(UnexpectedEOF(
          "Unexpected end-of-file after '.'. Expected to see an identifier.".to_owned(),
        )),
      };

      let (line_start, col_start, _, _) = extract_location(&node);
      let col_end = col + value.len();

      let child_node = Node::Identifier {
        line,
        col_start: col,
        col_end,
        name: to_string(value),
        inferred_type: NodeType::Unknown,
      };

      self.advance(1);

      node = Node::Chain {
        line_start,
        line_end: line,
        col_start,
        col_end,
        object: Box::new(node),
        property: Box::new(child_node),
      }
    }

    ParseResult::Ok(node)
  }

  fn parse_parenthetical(&mut self) -> ParseResult {
    let (line_start, col_start) = match self.current_token() {
      Some(&Token::LeftParen { line, col }) => (line, col),
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

    let (line_end, col_end) = match self.current_token() {
      Some(&Token::RightParen { line, col }) => (line, col),
      _ => return ParseResult::Error(UnclosedParentheses(self.index))
    };

    self.advance(1);

    if inner_exprs.len() == 1 {
      return ParseResult::Ok(Node::Grouping {
        line_start,
        col_start,
        line_end,
        col_end,
        expr: Box::new(inner_exprs[0].clone()),
        inferred_type: NodeType::Unknown,
      });
    }

    ParseResult::Ok(Node::Tuple {
      line_start,
      col_start,
      line_end,
      col_end,
      entries: inner_exprs,
      inferred_type: NodeType::Unknown,
    })
  }

  fn parse_any_calls_after_result(&mut self, previous: ParseResult) -> ParseResult {
    let mut current = previous.clone();
    let mut result = previous.clone();

    while let ParseResult::Ok(node) = current {
      let (line_start, _, col_start, _) = extract_location(&node);

      if let Some(&Token::LeftParen { line, .. }) = self.current_token() {
        if line != line_start {
          break
        }

        match self.parse_parenthetical() {
          ParseResult::Ok(Node::Tuple {
            line_end,
            col_end,
            entries,
            ..
          }) => {
            current = ParseResult::Ok(Node::Call {
              line_start,
              line_end,
              col_start,
              col_end,
              callee: Box::new(node),
              arguments: entries,
              inferred_type: NodeType::Unknown,
            });

            result = current.clone();
            continue
          },

          ParseResult::Ok(expr_in_parens) => {
            let (_, line_end, _, col_end) = extract_location(&expr_in_parens);

            current = ParseResult::Ok(Node::Call {
              line_start,
              line_end,
              col_start,
              col_end,
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
    let parsed = match self.current_token() {
      Some(&Token::Identifier { .. }) => self.parse_identifier(),
      Some(&Token::LeftParen { .. }) => self.parse_parenthetical(),
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
