use crate::parse_error::{ParseError, ParseErrorKind::*};
use crate::tokens::{Token, Token::*};
use std::collections::HashMap;

pub struct Tokenizer<'a> {
  source: &'a Vec<u8>,
  source_length: usize,
}

pub type TokenList = Vec<Token>;
pub type CommentMap = HashMap<usize, Token>;
pub type TokenizeResult = (TokenList, CommentMap);

impl<'a> Tokenizer<'a> {
  pub fn from_source(source: &'a Vec<u8>) -> Tokenizer {
    let length = source.len();

    return Tokenizer {
      source,
      source_length: length,
    };
  }

  pub fn collect_tokens(&mut self) -> (TokenList, CommentMap, Vec<ParseError>) {
    let source = self.source;
    let length = self.source_length;

    let mut tokens = Vec::new();
    let mut comments = HashMap::new();
    let mut errors = Vec::new();

    let mut index = 0;
    let mut line = 0;
    let mut expect_import_path = false;

    let mut string_stack = Vec::new();
    let mut interpolation_stack = Vec::new();
    let mut brace_depth = 0;

    // We iterate through all chars in a single loop, appending tokens as we find them.
    // The trickiest parts here are related to string interpolations, since they can
    // be nested arbitrarily deep (e.g. "hello $("Ms. $(name)")"). These parts are
    // commented below.
    'main_loop: while index < length {
      let start_index = index;
      let byte = source[start_index];

      if string_stack.is_empty() && byte == b'"' {
        // If the string stack is empty and byte is ", we are at the beginning of
        // a brand new string. Save the start index and advance.
        string_stack.push(index);
        index += 1;
        continue;
      }

      if !string_stack.is_empty() {
        // If the string stack is not empty, we're somewhere inside a string (maybe
        // in an interpolation, though). We must check if we need to end the string,
        // start/end an interpolation, or just carry on.

        if byte == b'"' && string_stack.len() == interpolation_stack.len() {
          // If the two stacks have the same size, we must be inside of an interpolation,
          // so the " indicates the beginning of a nested string literal. Save the index
          // in the string stack and advance.
          string_stack.push(index);
          index += 1;
          continue;
        }

        if byte == b'"' {
          let is_escaped = index > 0 && source[index - 1] == b'\\';

          if !is_escaped {
            // Here, the " must indicate the end of a string literal section. Pop from
            // the string stack, add a new token, then advance.
            let start_index = string_stack.pop().unwrap();

            tokens.push(StringLiteral(start_index + 1, index));

            index += 1;
            continue;
          }
        }

        if byte == b'$' && start_index + 1 < length && source[start_index + 1] == b'(' {
          // We must be at the beginning of an interpolation, so create a token for
          // the string literal portion leading up to the interpolation, one for the
          // interpolation start, and add to the interpolation stack.
          let string_start_index = string_stack.last().unwrap();

          tokens.push(StringLiteral(string_start_index + 1, index));
          tokens.push(InterpolationStart(start_index + 1, index + 2));

          interpolation_stack.push(index);
          index += 2;
          continue;
        }

        if byte == b')' {
          // We must be at the end of an interpolation, so make a token for it and
          // fix the index on the last string in the string stack so that it starts
          // here. Decrease the interpolation stack.
          tokens.push(InterpolationEnd(index, index + 1));

          string_stack.pop();
          string_stack.push(index);

          interpolation_stack.pop();
          index += 1;
          continue;
        }

        if string_stack.len() > interpolation_stack.len() {
          // If the string stack is larger than the interpolation stack, we must be
          // inside of a string literal portion. Just advance past this char so we can
          // include it in the string later.
          index += 1;
          continue;
        }

        // At this point, we must be inside an interpolation (not a string literal),
        // so continue to collect tokens as we would outside of a string.
      }

      if expect_import_path && is_path_char(byte) {
        let mut path_byte = byte;

        while is_path_char(path_byte) {
          index += 1;

          if index >= length {
            break;
          }

          path_byte = source[index];
        }

        tokens.push(ImportPath(start_index, index));
        expect_import_path = false;
        continue;
      }

      match byte {
        b' ' | b'\r' | b'\t' => {
          index += 1;
        }

        b'\n' => {
          index += 1;
          line += 1;
          tokens.push(LineBreak(start_index, index))
        }

        b'(' => {
          index += 1;
          tokens.push(LeftParen(start_index, index))
        }

        b')' => {
          index += 1;
          tokens.push(RightParen(start_index, index))
        }

        b'{' => {
          index += 1;
          brace_depth += 1;
          tokens.push(LeftBrace(start_index, index))
        }

        b'}' => {
          index += 1;
          brace_depth -= 1;
          tokens.push(RightBrace(start_index, index))
        }

        b'[' => {
          index += 1;
          tokens.push(LeftBracket(start_index, index))
        }

        b']' => {
          index += 1;
          tokens.push(RightBracket(start_index, index))
        }

        b',' => {
          index += 1;
          tokens.push(Comma(start_index, index))
        }

        b'_' if (index >= length - 1 || source[index + 1] != b'_') => {
          index += 1;
          tokens.push(Underscore(start_index, index))
        }

        b'$' if index < length - 1 && is_digit(source[index + 1]) => {
          index += 1;

          while index < length && is_digit(source[index]) {
            index += 1;
          }

          tokens.push(ParamPlaceholder(start_index, index));
        }

        _ if is_operator_char(byte) => {
          while index < length && is_operator_char(source[index]) {
            index += 1;
          }

          let value = &source[start_index..index];

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

          tokens.push(constructor(start_index, index));
        }

        b'#' => {
          while index < length && source[index] != b'\n' {
            index += 1;
          }

          comments.insert(line, Comment(start_index + 1, index));
        }

        _ if is_identifier_start_char(byte) => {
          while index < length && is_identifier_char(source[index]) {
            index += 1;
          }

          let value = &source[start_index..index];

          let constructor = match value {
            // These keywords cannot be used as identifiers anywhere:
            b"break" => KeywordBreak,
            b"let" => KeywordLet,
            b"match" => KeywordMatch,
            b"return" => KeywordReturn,

            // These are only considered keywords if they appear at the top level:
            b"def" if brace_depth == 0 => KeywordDef,
            b"enum" if brace_depth == 0 => KeywordEnum,
            b"alias" if brace_depth == 0 => KeywordAlias,
            b"as" if brace_depth == 0 => KeywordAs,
            b"intrinsic_def" if brace_depth == 0 => KeywordIntrinsicDef,
            b"intrinsic_type" if brace_depth == 0 => KeywordIntrinsicType,
            b"private" if brace_depth == 0 => KeywordPrivate,
            b"use" if brace_depth == 0 => KeywordUse,
            b"struct" if brace_depth == 0 => KeywordStruct,
            b"trait" if brace_depth == 0 => KeywordTrait,
            b"where" if brace_depth == 0 => KeywordWhere,

            // Anything else is just an identifier:
            _ => Identifier,
          };

          if constructor == KeywordUse {
            expect_import_path = true;
          }

          tokens.push(constructor(start_index, index));

          continue;
        }

        _ if is_digit(byte) => {
          if byte == b'0' {
            match source.get(index + 1) {
              Some(b'b') | Some(b'B') => {
                index += 2;

                while index < length && is_identifier_char(source[index]) {
                  if source[index] != b'0' && source[index] != b'1' {
                    let error_start = index;

                    while index < length && !source[index].is_ascii_whitespace() {
                      index += 1;
                    }

                    errors.push(ParseError {
                      pos: (error_start, index),
                      kind: InvalidBinaryDigit,
                    });

                    continue 'main_loop;
                  }

                  index += 1;
                }

                tokens.push(BinaryDigits(start_index, index));
                continue;
              }

              Some(b'x') | Some(b'X') => {
                index += 2;

                while index < length && is_identifier_char(source[index]) {
                  if !source[index].is_ascii_hexdigit() {
                    let error_start = index;

                    while index < length && !source[index].is_ascii_whitespace() {
                      index += 1;
                    }

                    errors.push(ParseError {
                      pos: (error_start, index),
                      kind: InvalidHexDigit,
                    });

                    continue 'main_loop;
                  }

                  index += 1;
                }

                tokens.push(HexDigits(start_index, index));
                continue;
              }

              Some(b'o') | Some(b'O') => {
                index += 2;

                while index < length && is_identifier_char(source[index]) {
                  if source[index] < 48 || source[index] > 55 {
                    let error_start = index;

                    while index < length && !source[index].is_ascii_whitespace() {
                      index += 1;
                    }

                    errors.push(ParseError {
                      pos: (error_start, index),
                      kind: InvalidOctalDigit,
                    });

                    continue 'main_loop;
                  }

                  index += 1;
                }

                tokens.push(OctalDigits(start_index, index));
                continue;
              }

              _ => {}
            }
          }

          while index < length && is_identifier_char(source[index]) {
            if !source[index].is_ascii_digit() {
              let error_start = index;

              while index < length && !source[index].is_ascii_whitespace() {
                index += 1;
              }

              errors.push(ParseError {
                pos: (error_start, index),
                kind: InvalidDecimalDigit,
              });

              continue 'main_loop;
            }

            index += 1;
          }

          tokens.push(DecimalDigits(start_index, index));
          continue;
        }

        _ => {
          index += 1;
          tokens.push(Unexpected(start_index, index))
        }
      };
    }

    if !interpolation_stack.is_empty() {
      let start_index = interpolation_stack.pop().unwrap();

      errors.push(ParseError {
        pos: (start_index, index),
        kind: UnclosedInterpolation,
      });
    }

    if !string_stack.is_empty() {
      let start_index = string_stack.pop().unwrap();

      errors.push(ParseError {
        pos: (start_index, start_index + 1),
        kind: UnclosedString,
      });
    }

    (tokens, comments, errors)
  }
}

fn is_identifier_start_char(byte: u8) -> bool {
  match byte {
    _ if byte.is_ascii_digit() => false,
    _ if byte.is_ascii_whitespace() => false,
    _ if byte.is_ascii_punctuation() => false,
    _ if byte.is_ascii_control() => false,
    _ => true,
  }
}

fn is_identifier_char(byte: u8) -> bool {
  match byte {
    _ if byte.is_ascii_whitespace() => false,
    _ if byte.is_ascii_punctuation() => false,
    _ if byte.is_ascii_control() => false,
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
