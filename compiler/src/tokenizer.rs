use crate::tokens::{Token, Token::*};
use std::collections::HashMap;
use crate::errors::{TokenizeError, TokenizeError::*};

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

  pub fn collect_tokens(&mut self) -> Result<TokenizeResult, TokenizeError> {
    let source = self.source;
    let length = self.source_length;

    let mut tokens = Vec::new();
    let mut comments = HashMap::new();

    let mut index = 0;
    let mut line = 0;

    let mut string_stack = Vec::new();
    let mut interpolation_stack = Vec::new();

    // We iterate through all chars in a single loop, appending tokens as we find them.
    // The trickiest parts here are related to string interpolations, since they can
    // be nested arbitrarily deep (e.g. "hello $("Ms. $(name)")"). These parts are
    // commented below.
    while index < length {
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
          // Here, the " must indicate the end of a string literal section. Pop from
          // the string stack, add a new token, then advance.
          let start_index = string_stack.pop().unwrap();

          tokens.push(StringLiteral(start_index + 1, index));

          index += 1;
          continue;
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
          tokens.push(LeftBrace(start_index, index))
        }

        b'}' => {
          index += 1;
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

        b'.' => {
          index += 1;
          tokens.push(Dot(start_index, index))
        }

        b'|' => {
          index += 1;
          tokens.push(Pipe(start_index, index))
        }

        b'=' => match source.get(index + 1) {
          Some(b'>') => {
            index += 2;
            tokens.push(DoubleArrow(start_index, index))
          }

          _ => {
            index += 1;
            tokens.push(Equals(start_index, index))
          }
        },

        b'-' => match source.get(index + 1) {
          Some(b'>') => {
            index += 2;
            tokens.push(Arrow(start_index, index))
          }

          _ => {
            index += 1;
            tokens.push(Minus(start_index, index))
          }
        },

        b':' => match source.get(index + 1) {
          Some(b'=') => {
            index += 2;
            tokens.push(ColonEquals(start_index, index))
          }

          Some(b':') => {
            index += 2;
            tokens.push(DoubleColon(start_index, index))
          }

          _ => {
            index += 1;
            tokens.push(Colon(start_index, index))
          }
        },

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

          match value[0] {
            b'l' if value == "let".as_bytes() => tokens.push(KeywordLet(start_index, index)),
            b'm' if value == "match".as_bytes() => tokens.push(KeywordMatch(start_index, index)),
            b'd' if value == "def".as_bytes() => tokens.push(KeywordDef(start_index, index)),
            b't' if value == "type".as_bytes() => tokens.push(KeywordType(start_index, index)),
            b'u' if value == "use".as_bytes() => tokens.push(KeywordUse(start_index, index)),
            b'a' if value == "as".as_bytes() => tokens.push(KeywordAs(start_index, index)),
            _ => tokens.push(Identifier(start_index, index))
          }

          continue;
        }

        _ if is_digit(byte) => {
          if byte == b'0' {
            match source.get(index + 1) {
              Some(b'b') | Some(b'B') => {
                index += 2;

                while index < length && is_identifier_char(source[index]) {
                  if source[index] != b'0' && source[index] != b'1' {
                    return Err(InvalidBinaryDigitError(index, index + 1))
                  }

                  index += 1;
                }

                tokens.push(BinaryDigits(start_index, index));
                continue;
              },

              Some(b'x') | Some(b'X') => {
                index += 2;

                while index < length && is_identifier_char(source[index]) {
                  if !source[index].is_ascii_hexdigit() {
                    return Err(InvalidHexDigitError(index, index + 1))
                  }

                  index += 1;
                }

                tokens.push(HexDigits(start_index, index));
                continue;
              },

              Some(b'o') | Some(b'O') => {
                index += 2;

                while index < length && is_identifier_char(source[index]) {
                  if source[index] < 48 || source[index] > 55 {
                    return Err(InvalidOctalDigitError(index, index + 1))
                  }

                  index += 1;
                }

                tokens.push(OctalDigits(start_index, index));
                continue;
              },

              _ => {}
            }
          }

          while index < length && is_identifier_char(source[index]) {
            if !source[index].is_ascii_digit() {
              return Err(InvalidDecimalDigitError(index, index + 1))
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
      return Err(UnclosedInterpolationError(start_index, index))
    }

    if !string_stack.is_empty() {
      let start_index = string_stack.pop().unwrap();
      return Err(UnclosedStringError(start_index, index))
    }

    Ok((tokens, comments))
  }
}

fn is_identifier_start_char(byte: u8) -> bool {
  match byte {
    b'a'...b'z' | b'A'...b'Z' => true,
    _ => false
  }
}

fn is_identifier_char(byte: u8) -> bool {
  match byte {
    b'a'...b'z' | b'A'...b'Z' | b'0'...b'9' => true,
    _ => false
  }
}

fn is_digit(byte: u8) -> bool {
  match byte {
    b'0'...b'9' => true,
    _ => false
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::assert_tokens_snapshot;
  use insta::assert_snapshot;

  assert_tokens_snapshot!(
    empty,
    ""
  );

  assert_tokens_snapshot!(
    identifers,
    "hello world"
  );

  assert_tokens_snapshot!(
    numbers,
    "hello 1 47 wow"
  );

  assert_tokens_snapshot!(
    binary_numbers,
    "0b101 0b00 0b1 0B0 0b00100"
  );

  assert_tokens_snapshot!(
    hex_numbers,
    "0x101 0x0 0xfacade 0X47"
  );

  assert_tokens_snapshot!(
    octal_numbers,
    "0o101 0o0 0o47 0O47"
  );

  assert_tokens_snapshot!(
    comment_before,
    "# comment \nok"
  );

  assert_tokens_snapshot!(
    comment_same_line,
    "ok #comment"
  );

  assert_tokens_snapshot!(
    comment_after,
    "ok \n\n#comment"
  );

  assert_tokens_snapshot!(
    symbols,
    "{ . } ( , ) : [ :: ] := = => ->"
  );

  assert_tokens_snapshot!(
    unexpected,
    "(@$@)"
  );

  assert_tokens_snapshot!(
    strings_without_interpolations,
    "\"hello\" \"\" \"world\""
  );

  assert_tokens_snapshot!(
    strings_with_interpolations,
    "\"hello $(name)!\" nice \"$(str)\""
  );

  assert_tokens_snapshot!(
    strings_with_nested_interpolations,
    "\"hello $(name \"inner $(o)\" wow)!\""
  );
}
