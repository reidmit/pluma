use crate::parse_error::{ParseError, ParseErrorKind::*};
use crate::tokens::{Token, Token::*};
use std::collections::HashMap;

pub type CommentMap = HashMap<usize, Token>;

pub struct Tokenizer<'a> {
  source: &'a Vec<u8>,
  length: usize,
  index: usize,
  line: usize,
  expect_import_path: bool,
  string_stack: Vec<usize>,
  interpolation_stack: Vec<usize>,
  brace_depth: i32,
  comments: CommentMap,
  errors: Vec<ParseError>,
  next_token: Option<Token>,
}

impl<'a> Tokenizer<'a> {
  pub fn from_source(source: &'a Vec<u8>) -> Self {
    let length = source.len();

    return Tokenizer {
      source,
      length,
      index: 0,
      line: 0,
      expect_import_path: false,
      string_stack: Vec::new(),
      interpolation_stack: Vec::new(),
      brace_depth: 0,
      comments: HashMap::new(),
      errors: Vec::new(),
      next_token: None,
    };
  }
}

impl<'a> Iterator for Tokenizer<'a> {
  type Item = Token;

  fn next(&mut self) -> Option<Token> {
    if self.index >= self.length {
      return None;
    }

    if let Some(next_token) = self.next_token {
      self.next_token = None;
      return Some(next_token);
    }

    // We iterate through all chars in a single loop, appending tokens as we find them.
    // The trickiest parts here are related to string interpolations, since they can
    // be nested arbitrarily deep (e.g. "hello $("Ms. $(name)")"). These parts are
    // commented below.
    'main_loop: while self.index < self.length {
      let start_index = self.index;
      let byte = self.source[start_index];

      if self.string_stack.is_empty() && byte == b'"' {
        // If the string stack is empty and byte is ", we are at the beginning of
        // a brand new string. Save the start index and advance.
        self.string_stack.push(self.index);
        self.index += 1;
        continue;
      }

      if !self.string_stack.is_empty() {
        // If the string stack is not empty, we're somewhere inside a string (maybe
        // in an interpolation, though). We must check if we need to end the string,
        // start/end an interpolation, or just carry on.

        if byte == b'"' && self.string_stack.len() == self.interpolation_stack.len() {
          // If the two stacks have the same size, we must be inside of an interpolation,
          // so the " indicates the beginning of a nested string literal. Save the index
          // in the string stack and advance.
          self.string_stack.push(self.index);
          self.index += 1;
          continue;
        }

        if byte == b'"' {
          let is_escaped = self.index > 0 && self.source[self.index - 1] == b'\\';

          if !is_escaped {
            // Here, the " must indicate the end of a string literal section. Pop from
            // the string stack, add a new token, then advance.
            let start_index = self.string_stack.pop().unwrap() + 1;
            let end_index = self.index;
            self.index += 1;

            return Some(StringLiteral(start_index, end_index));
          }
        }

        if byte == b'$' && start_index + 1 < self.length && self.source[start_index + 1] == b'(' {
          // We must be at the beginning of an interpolation, so create a token for
          // the string literal portion leading up to the interpolation, one for the
          // interpolation start, and add to the interpolation stack.
          let string_start_index = self.string_stack.last().unwrap() + 1;
          let string_end_index = self.index;

          let interpolation_start_start_index = start_index + 1;
          let interpolation_start_end_index = self.index + 2;

          self.interpolation_stack.push(self.index);
          self.index += 2;

          self.next_token = Some(InterpolationStart(
            interpolation_start_end_index,
            interpolation_start_start_index,
          ));

          return Some(StringLiteral(string_start_index, string_end_index));
        }

        if self.interpolation_stack.len() > 0 && byte == b')' {
          // We must be at the end of an interpolation, so make a token for it and
          // fix the index on the last string in the string stack so that it starts
          // here. Decrease the interpolation stack.
          let start_index = self.index;
          let end_index = self.index + 1;

          self.string_stack.pop();
          self.string_stack.push(self.index);

          self.interpolation_stack.pop();
          self.index += 1;

          return Some(InterpolationEnd(start_index, end_index));
        }

        if self.string_stack.len() > self.interpolation_stack.len() {
          // If the string stack is larger than the interpolation stack, we must be
          // inside of a string literal portion. Just advance past this char so we can
          // include it in the string later.
          self.index += 1;
          continue;
        }

        // At this point, we must be inside an interpolation (not a string literal),
        // so continue to collect tokens as we would outside of a string.
      }

      if self.expect_import_path && is_path_char(byte) {
        let mut path_byte = byte;

        while is_path_char(path_byte) {
          self.index += 1;

          if self.index >= self.length {
            break;
          }

          path_byte = self.source[self.index];
        }

        self.expect_import_path = false;

        return Some(ImportPath(start_index, self.index));
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

        b',' => {
          self.index += 1;
          return Some(Comma(start_index, self.index));
        }

        b'_' if (self.index >= self.length - 1 || self.source[self.index + 1] != b'_') => {
          self.index += 1;
          return Some(Underscore(start_index, self.index));
        }

        b'$' if self.index < self.length - 1 && is_digit(self.source[self.index + 1]) => {
          self.index += 1;

          while self.index < self.length && is_digit(self.source[self.index]) {
            self.index += 1;
          }

          return Some(ParamPlaceholder(start_index, self.index));
        }

        _ if is_operator_char(byte) => {
          while self.index < self.length && is_operator_char(self.source[self.index]) {
            self.index += 1;
          }

          let value = &self.source[start_index..self.index];

          let constructor = match value {
            b"." => Dot,
            b"|" => Pipe,
            b"=>" => DoubleArrow,
            b"=" => Equals,
            b"->" => Arrow,
            b"::" => DoubleColon,
            b":" => Colon,
            b"<" => LeftAngle,
            b">" => RightAngle,
            _ => Operator,
          };

          return Some(constructor(start_index, self.index));
        }

        b'#' => {
          while self.index < self.length && self.source[self.index] != b'\n' {
            self.index += 1;
          }

          self
            .comments
            .insert(self.line, Comment(start_index + 1, self.index));
        }

        _ if is_identifier_start_char(byte) => {
          while self.index < self.length && is_identifier_char(self.source[self.index]) {
            self.index += 1;
          }

          let value = &self.source[start_index..self.index];

          let constructor = match value {
            // These keywords cannot be used as identifiers anywhere:
            b"break" => KeywordBreak,
            b"let" => KeywordLet,
            b"match" => KeywordMatch,
            b"mut" => KeywordMut,
            b"return" => KeywordReturn,

            // These are only considered keywords if they appear at the top level:
            b"def" if self.brace_depth == 0 => KeywordDef,
            b"enum" if self.brace_depth == 0 => KeywordEnum,
            b"alias" if self.brace_depth == 0 => KeywordAlias,
            b"as" if self.brace_depth == 0 => KeywordAs,
            b"intrinsic_def" if self.brace_depth == 0 => KeywordIntrinsicDef,
            b"intrinsic_type" if self.brace_depth == 0 => KeywordIntrinsicType,
            b"private" if self.brace_depth == 0 => KeywordPrivate,
            b"use" if self.brace_depth == 0 => KeywordUse,
            b"struct" if self.brace_depth == 0 => KeywordStruct,
            b"trait" if self.brace_depth == 0 => KeywordTrait,
            b"where" if self.brace_depth == 0 => KeywordWhere,

            // Anything else is just an identifier:
            _ => Identifier,
          };

          if constructor == KeywordUse {
            self.expect_import_path = true;
          }

          return Some(constructor(start_index, self.index));
        }

        _ if is_digit(byte) => {
          if byte == b'0' {
            match self.source.get(self.index + 1) {
              Some(b'b') | Some(b'B') => {
                self.index += 2;

                while self.index < self.length && is_identifier_char(self.source[self.index]) {
                  if self.source[self.index] != b'0' && self.source[self.index] != b'1' {
                    let error_start = self.index;

                    while self.index < self.length && !self.source[self.index].is_ascii_whitespace()
                    {
                      self.index += 1;
                    }

                    self.errors.push(ParseError {
                      pos: (error_start, self.index),
                      kind: InvalidBinaryDigit,
                    });

                    continue 'main_loop;
                  }

                  self.index += 1;
                }

                return Some(BinaryDigits(start_index, self.index));
              }

              Some(b'x') | Some(b'X') => {
                self.index += 2;

                while self.index < self.length && is_identifier_char(self.source[self.index]) {
                  if !self.source[self.index].is_ascii_hexdigit() {
                    let error_start = self.index;

                    while self.index < self.length && !self.source[self.index].is_ascii_whitespace()
                    {
                      self.index += 1;
                    }

                    self.errors.push(ParseError {
                      pos: (error_start, self.index),
                      kind: InvalidHexDigit,
                    });

                    continue 'main_loop;
                  }

                  self.index += 1;
                }

                return Some(HexDigits(start_index, self.index));
              }

              Some(b'o') | Some(b'O') => {
                self.index += 2;

                while self.index < self.length && is_identifier_char(self.source[self.index]) {
                  if self.source[self.index] < 48 || self.source[self.index] > 55 {
                    let error_start = self.index;

                    while self.index < self.length && !self.source[self.index].is_ascii_whitespace()
                    {
                      self.index += 1;
                    }

                    self.errors.push(ParseError {
                      pos: (error_start, self.index),
                      kind: InvalidOctalDigit,
                    });

                    continue 'main_loop;
                  }

                  self.index += 1;
                }

                return Some(OctalDigits(start_index, self.index));
              }

              _ => {}
            }
          }

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

          return Some(DecimalDigits(start_index, self.index));
        }

        _ => {
          self.index += 1;
          return Some(Unexpected(start_index, self.index));
        }
      };
    }

    if !self.interpolation_stack.is_empty() {
      let start_index = self.interpolation_stack.pop().unwrap();

      self.errors.push(ParseError {
        pos: (start_index, self.index),
        kind: UnclosedInterpolation,
      });
    }

    if !self.string_stack.is_empty() {
      let start_index = self.string_stack.pop().unwrap();

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
    _ if byte.is_ascii_whitespace() => false,
    _ if byte.is_ascii_control() => false,
    _ if is_operator_char(byte) => false,
    b'"' => false,
    b'#' => false,
    b'$' => false,
    b'\'' => false,
    b'(' => false,
    b')' => false,
    b',' => false,
    b';' => false,
    b'`' => false,
    b'[' => false,
    b']' => false,
    b'^' => false,
    b'{' => false,
    b'}' => false,
    _ => true,
  }
}

fn is_digit(byte: u8) -> bool {
  match byte {
    b'0'..=b'9' => true,
    _ => false,
  }
}

fn is_operator_char(byte: u8) -> bool {
  match byte {
    b':' => true,
    b'|' => true,
    b'.' => true,
    b'*' => true,
    b'/' => true,
    b'+' => true,
    b'-' => true,
    b'=' => true,
    b'<' => true,
    b'>' => true,
    b'~' => true,
    b'!' => true,
    b'%' => true,
    b'&' => true,
    b'@' => true,
    b'^' => true,
    b'?' => true,
    _ => false,
  }
}

fn is_path_char(byte: u8) -> bool {
  match byte {
    b'\\' | b'?' | b'%' | b'*' | b':' | b'"' | b'<' | b'>' => false,
    b if b.is_ascii_whitespace() => false,
    _ => true,
  }
}
