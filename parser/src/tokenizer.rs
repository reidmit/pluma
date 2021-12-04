use crate::parse_error::{ParseError, ParseErrorKind::*};
use crate::tokens::{Token, Token::*};
use std::collections::HashMap;

macro_rules! read_string {
  ($self:ident, $start:expr, $end:expr) => {
    String::from_utf8($self.source[$start..$end].to_vec()).expect("not utf-8");
  };
}

pub struct Tokenizer<'a> {
  pub comments: HashMap<usize, String>,
  source: &'a Vec<u8>,
  length: usize,
  index: usize,
  line: usize,
  string_start: Option<usize>,
  brace_depth: i32,
  errors: Vec<ParseError>,
  next_token: Option<Token>,
  peek_queue: Vec<Token>,
}

impl<'a> Tokenizer<'a> {
  pub fn from_source(source: &'a Vec<u8>) -> Self {
    let length = source.len();

    return Tokenizer {
      source,
      length,
      index: 0,
      line: 1,
      string_start: None,
      brace_depth: 0,
      comments: HashMap::new(),
      errors: Vec::new(),
      next_token: None,
      peek_queue: Vec::with_capacity(2),
    };
  }

  pub fn peek(&mut self) -> Option<Token> {
    let peeked_token = self.next();

    if let Some(token) = peeked_token {
      self.peek_queue.insert(0, token);
    }

    peeked_token
  }
}

impl<'a> Iterator for Tokenizer<'a> {
  type Item = Token;

  fn next(&mut self) -> Option<Token> {
    if !self.peek_queue.is_empty() {
      return self.peek_queue.pop();
    }

    if self.index >= self.length {
      return None;
    }

    if let Some(next_token) = self.next_token {
      self.next_token = None;
      return Some(next_token);
    }

    // We iterate through all chars in a single loop, appending tokens as we find them.
    'main_loop: while self.index < self.length {
      let start_index = self.index;
      let byte = self.source[start_index];

      if self.string_start.is_none() && byte == b'"' {
        // If the string start is empty and byte is ", we are at the beginning of
        // a brand new string. Save the start index and advance.
        self.string_start = Some(self.index);
        self.index += 1;
        continue;
      }

      if let Some(string_start) = self.string_start {
        // If the string stack is not empty, we're somewhere inside a string (maybe
        // in an interpolation, though). We must check if we need to end the string,
        // start/end an interpolation, or just carry on.

        if byte == b'"' {
          let is_escaped = self.index > 0 && self.source[self.index - 1] == b'\\';

          if !is_escaped {
            // Here, the " must indicate the end of a string literal section. Grab
            // the string start, add a new token, and advance.
            let start_index = string_start + 1;
            let end_index = self.index;
            self.index += 1;
            self.string_start = None;

            return Some(StringLiteral(start_index, end_index));
          }
        }

        self.index += 1;
      }

      match byte {
        b' ' | b'\r' | b'\t' => {
          self.index += 1;
        }

        b'\n' => {
          self.index += 1;
          self.line += 1;
          return Some(LineBreak(start_index, self.index));
        }

        b'(' => {
          self.index += 1;
          return Some(LeftParen(start_index, self.index));
        }

        b')' => {
          self.index += 1;
          return Some(RightParen(start_index, self.index));
        }

        b'{' => {
          self.index += 1;
          self.brace_depth += 1;
          return Some(LeftBrace(start_index, self.index));
        }

        b'}' => {
          self.index += 1;
          self.brace_depth -= 1;
          return Some(RightBrace(start_index, self.index));
        }

        b'[' => {
          self.index += 1;
          return Some(LeftBracket(start_index, self.index));
        }

        b']' => {
          self.index += 1;
          return Some(RightBracket(start_index, self.index));
        }

        b'/' => {
          self.index += 1;
          return Some(ForwardSlash(start_index, self.index));
        }

        b'%' => {
          self.index += 1;
          return Some(Percent(start_index, self.index));
        }

        b'-' => {
          self.index += 1;

          if self.source[self.index] == b'>' {
            self.index += 1;
            return Some(Arrow(start_index, self.index));
          }

          return Some(Minus(start_index, self.index));
        }

        b'+' => {
          self.index += 1;

          if self.source[self.index] == b'+' {
            self.index += 1;
            return Some(DoublePlus(start_index, self.index));
          }

          return Some(Plus(start_index, self.index));
        }

        b',' => {
          self.index += 1;
          return Some(Comma(start_index, self.index));
        }

        b'^' => {
          self.index += 1;
          return Some(Caret(start_index, self.index));
        }

        b'~' => {
          self.index += 1;
          return Some(Tilde(start_index, self.index));
        }

        b'?' => {
          self.index += 1;
          return Some(Question(start_index, self.index));
        }

        b'_' => {
          self.index += 1;
          return Some(Underscore(start_index, self.index));
        }

        b'!' => {
          self.index += 1;

          if self.source[self.index] == b'=' {
            self.index += 1;
            return Some(BangEqual(start_index, self.index));
          }

          return Some(Bang(start_index, self.index));
        }

        b'=' => {
          self.index += 1;

          if self.source[self.index] == b'=' {
            self.index += 1;
            return Some(DoubleEqual(start_index, self.index));
          }

          if self.source[self.index] == b'>' {
            self.index += 1;
            return Some(DoubleArrow(start_index, self.index));
          }

          return Some(Equal(start_index, self.index));
        }

        b'*' => {
          self.index += 1;

          if self.source[self.index] == b'*' {
            self.index += 1;
            return Some(DoubleStar(start_index, self.index));
          }

          return Some(Star(start_index, self.index));
        }

        b'.' => {
          self.index += 1;
          return Some(Dot(start_index, self.index));
        }

        b'&' => {
          self.index += 1;

          if self.source[self.index] == b'&' {
            self.index += 1;
            return Some(DoubleAnd(start_index, self.index));
          }

          return Some(And(start_index, self.index));
        }

        b'|' => {
          self.index += 1;

          if self.source[self.index] == b'|' {
            self.index += 1;
            return Some(DoublePipe(start_index, self.index));
          }

          return Some(Pipe(start_index, self.index));
        }

        b':' => {
          self.index += 1;

          if self.source[self.index] == b':' {
            self.index += 1;
            return Some(DoubleColon(start_index, self.index));
          }

          return Some(Colon(start_index, self.index));
        }

        b'<' => {
          self.index += 1;

          if self.source[self.index] == b'=' {
            self.index += 1;
            return Some(LeftAngleEqual(start_index, self.index));
          }

          if self.source[self.index] == b'<' {
            self.index += 1;
            return Some(DoubleLeftAngle(start_index, self.index));
          }

          return Some(LeftAngle(start_index, self.index));
        }

        b'>' => {
          self.index += 1;

          if self.source[self.index] == b'=' {
            self.index += 1;
            return Some(RightAngleEqual(start_index, self.index));
          }

          if self.source[self.index] == b'>' {
            self.index += 1;
            return Some(DoubleRightAngle(start_index, self.index));
          }

          return Some(RightAngle(start_index, self.index));
        }

        b'#' => {
          while self.index < self.length && self.source[self.index] != b'\n' {
            self.index += 1;
          }

          let comment = read_string!(self, start_index + 1, self.index);

          self.comments.insert(self.line, comment);
        }

        _ if is_identifier_start_char(byte) => {
          while self.index < self.length && is_identifier_char(self.source[self.index]) {
            self.index += 1;
          }

          let value = &self.source[start_index..self.index];

          let constructor = match value {
            // These keywords cannot be used as identifiers anywhere:
            b"let" => KeywordLet,
            b"match" => KeywordMatch,
            b"mut" => KeywordMut,

            // These are only considered keywords if they appear at the top level:
            b"enum" if self.brace_depth == 0 => KeywordEnum,
            b"alias" if self.brace_depth == 0 => KeywordAlias,
            b"struct" if self.brace_depth == 0 => KeywordStruct,
            b"trait" if self.brace_depth == 0 => KeywordTrait,
            b"where" if self.brace_depth == 0 => KeywordWhere,

            // Anything else is just an identifier:
            _ => Identifier,
          };

          return Some(constructor(start_index, self.index));
        }

        _ if is_digit(byte) => {
          while self.index < self.length && is_identifier_char(self.source[self.index]) {
            if !self.source[self.index].is_ascii_digit() {
              let error_start = self.index;

              while self.index < self.length && !self.source[self.index].is_ascii_whitespace() {
                self.index += 1;
              }

              self.errors.push(ParseError {
                pos: (error_start, self.index),
                kind: InvalidDecimalDigit,
              });

              continue 'main_loop;
            }

            self.index += 1;
          }

          return Some(Digits(start_index, self.index));
        }

        _ => {
          self.index += 1;
          return Some(Unexpected(start_index, self.index));
        }
      };
    }

    if let Some(start_index) = self.string_start {
      self.errors.push(ParseError {
        pos: (start_index, start_index + 1),
        kind: UnclosedString,
      });
    }

    None
  }
}

fn is_identifier_start_char(byte: u8) -> bool {
  match byte {
    _ if is_digit(byte) => false,
    _ => is_identifier_char(byte),
  }
}

fn is_identifier_char(byte: u8) -> bool {
  match byte {
    b'a'..=b'z' => true,
    b'A'..=b'A' => true,
    b'-' => true,
    _ if is_digit(byte) => true,
    _ => false,
  }
}

fn is_digit(byte: u8) -> bool {
  match byte {
    b'0'..=b'9' => true,
    _ => false,
  }
}
