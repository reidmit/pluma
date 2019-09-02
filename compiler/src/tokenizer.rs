use crate::tokens::Token;
use crate::errors::TokenizeError;

pub struct Tokenizer<'a> {
  source: &'a Vec<u8>,
  preserve_comments: bool,
  source_length: usize,
}

fn is_identifier_start_char(byte: u8) -> bool {
  (byte >= b'a' && byte <= b'z') || (byte >= b'A' && byte <= b'Z')
}

fn is_identifier_char(byte: u8) -> bool {
  (byte >= b'a' && byte <= b'z') || (byte >= b'A' && byte <= b'Z') || (byte >= b'0' && byte <= b'9')
}

fn is_digit(byte: u8) -> bool {
  byte >= b'0' && byte <= b'9'
}

impl<'a> Tokenizer<'a> {
  pub fn from_source(source: &'a Vec<u8>) -> Tokenizer<'a> {
    let length = source.len();

    return Tokenizer {
      source: source,
      preserve_comments: true,
      source_length: length,
    };
  }

  pub fn collect_tokens(&mut self) -> Result<Vec<Token<'a>>, TokenizeError> {
    let source = self.source;
    let length = self.source_length;

    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 0;
    let mut line_start_index = 0;

    let mut string_stack = Vec::new();
    let mut interpolation_stack = 0;

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
        string_stack.push((line, index - line_start_index, index));
        index += 1;
        continue;
      }

      if !string_stack.is_empty() {
        // If the string stack is not empty, we're somewhere inside a string (maybe
        // in an interpolation, though). We must check if we need to end the string,
        // start/end an interpolation, or just carry on.

        if byte == b'"' && string_stack.len() == interpolation_stack {
          // If the two stacks have the same size, we must be inside of an interpolation,
          // so the " indicates the beginning of a nested string literal. Save the index
          // in the string stack and advance.
          string_stack.push((line, index - line_start_index, index));
          index += 1;
          continue;
        }

        if byte == b'"' {
          // Here, the " must indicate the end of a string literal section. Pop from
          // the string stack, add a new token, then advance.
          let (line_start, col_start, offset) = string_stack.pop().unwrap();

          println!("str ending, {}, {}, {}", line_start, col_start, offset);

          tokens.push(Token::String {
            line_start,
            col_start,
            line_end: line,
            col_end: index + 1 - line_start_index,
            value: &source[offset + 1..index],
          });

          index += 1;
          continue;
        }

        if byte == b'$' && start_index + 1 < length && source[start_index + 1] == b'(' {
          // We must be at the beginning of an interpolation, so create a token for
          // the string literal portion leading up to the interpolation, one for the
          // interpolation start, and add to the interpolation stack.
          let (line_start, col_start, offset) = string_stack.last().unwrap();

          tokens.push(Token::String {
            line_start: *line_start,
            col_start: *col_start,
            line_end: line,
            col_end: index - line_start_index,
            value: &source[offset + 1..index],
          });

          tokens.push(Token::InterpolationStart {
            line: line,
            col: index - line_start_index,
          });

          index += 2;
          interpolation_stack += 1;
          continue;
        }

        if byte == b')' {
          // We must be at the end of an interpolation, so make a token for it and
          // fix the index on the last string in the string stack so that it starts
          // here. Decrease the interpolation stack.
          tokens.push(Token::InterpolationEnd {
            line: line,
            col: index - line_start_index,
          });

          string_stack.pop();
          string_stack.push((line, index - line_start_index + 1, index));

          index += 1;
          interpolation_stack -= 1;
          continue;
        }

        if string_stack.len() > interpolation_stack {
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
          line_start_index = index;
        }

        b'(' => {
          index += 1;

          tokens.push(Token::LeftParen {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b')' => {
          index += 1;

          tokens.push(Token::RightParen {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b'{' => {
          index += 1;

          tokens.push(Token::LeftBrace {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b'}' => {
          index += 1;

          tokens.push(Token::RightBrace {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b'[' => {
          index += 1;

          tokens.push(Token::LeftBracket {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b']' => {
          index += 1;

          tokens.push(Token::RightBracket {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b',' => {
          index += 1;

          tokens.push(Token::Comma {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b'.' => {
          index += 1;

          tokens.push(Token::Dot {
            line: line,
            col: start_index - line_start_index,
          })
        }

        b'=' => match source.get(index + 1) {
          Some(b'>') => {
            index += 2;

            tokens.push(Token::DoubleArrow {
              line: line,
              col: start_index - line_start_index,
            })
          }

          _ => {
            index += 1;

            tokens.push(Token::Equals {
              line: line,
              col: start_index - line_start_index,
            })
          }
        },

        b'-' => match source.get(index + 1) {
          Some(b'>') => {
            index += 2;

            tokens.push(Token::Arrow {
              line: line,
              col: start_index - line_start_index,
            })
          }

          _ => {
            index += 1;

            tokens.push(Token::Minus {
              line: line,
              col: start_index - line_start_index,
            })
          }
        },

        b':' => match source.get(index + 1) {
          Some(b'=') => {
            index += 2;

            tokens.push(Token::ColonEquals {
              line: line,
              col: start_index - line_start_index,
            })
          }

          Some(b':') => {
            index += 2;

            tokens.push(Token::DoubleColon {
              line: line,
              col: start_index - line_start_index,
            })
          }

          _ => {
            index += 1;

            tokens.push(Token::Colon {
              line: line,
              col: start_index - line_start_index,
            })
          }
        },

        b'#' => {
          while index < length && source[index] != b'\n' {
            index += 1;
          }

          if self.preserve_comments {
            tokens.push(Token::Comment {
              line: line,
              col: start_index - line_start_index,
              value: &source[start_index + 1..index],
            })
          }
        }

        _ if is_identifier_start_char(byte) => {
          while index < length && is_identifier_char(source[index]) {
            index += 1;
          }

          tokens.push(Token::Identifier {
            line: line,
            col: start_index - line_start_index,
            value: &source[start_index..index],
          });

          continue;
        }

        _ if is_digit(byte) => {
          while index < length && is_digit(source[index]) {
            index += 1;
          }

          tokens.push(Token::Number {
            line: line,
            col: start_index - line_start_index,
            value: &source[start_index..index],
          });

          continue;
        }

        _ => {
          index += 1;

          tokens.push(Token::Unexpected {
            line: line,
            col: start_index - line_start_index,
          })
        }
      };
    }

    if interpolation_stack > 0 {
      return Err(TokenizeError{
        message: "Unclosed interpolation".to_owned(),
        line: 0, // TODO
        col: 0, // TODO
      });
    }

    if !string_stack.is_empty() {
      let (line_start, col_start, _) = string_stack.pop().unwrap();

      return Err(TokenizeError{
        message: "Unclosed string".to_owned(),
        line: line_start,
        col: col_start,
      });
    }

    Ok(tokens)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::assert_tokens_snapshot;
  use insta::assert_snapshot;

  assert_tokens_snapshot!(
    no_tokens,
    ""
  );

  assert_tokens_snapshot!(
    identifer_tokens,
    "hello world"
  );

  assert_tokens_snapshot!(
    number_tokens,
    "hello 1 47 wow"
  );

  assert_tokens_snapshot!(
    comment_tokens,
    "# o #nice\n# hello\ntest #same-line"
  );

  assert_tokens_snapshot!(
    symbol_tokens,
    "{ . } ( , ) : [ :: ] := = => ->"
  );

  assert_tokens_snapshot!(
    unexpected_tokens,
    "(@$@)"
  );

  assert_tokens_snapshot!(
    string_tokens_without_interpolations,
    "\"hello\" \"\" \"world\""
  );

  assert_tokens_snapshot!(
    string_tokens_with_interpolations,
    "\"hello $(name)!\" nice \"$(str)\""
  );

  assert_tokens_snapshot!(
    string_tokens_with_nested_interpolations,
    "\"hello $(name \"inner $(o)\" wow)!\""
  );
}
