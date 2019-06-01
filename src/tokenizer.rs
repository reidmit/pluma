use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
  Unexpected {
    line: usize,
    col: usize,
  },
  LeftParen {
    line: usize,
    col: usize,
  },
  RightParen {
    line: usize,
    col: usize,
  },
  LeftBrace {
    line: usize,
    col: usize,
  },
  RightBrace {
    line: usize,
    col: usize,
  },
  LeftBracket {
    line: usize,
    col: usize,
  },
  RightBracket {
    line: usize,
    col: usize,
  },
  Comma {
    line: usize,
    col: usize,
  },
  Dot {
    line: usize,
    col: usize,
  },
  Colon {
    line: usize,
    col: usize,
  },
  Equals {
    line: usize,
    col: usize,
  },
  Minus {
    line: usize,
    col: usize,
  },
  Arrow {
    line: usize,
    col: usize,
  },
  DoubleArrow {
    line: usize,
    col: usize,
  },
  DoubleColon {
    line: usize,
    col: usize,
  },
  ColonEquals {
    line: usize,
    col: usize,
  },
  Identifier {
    line: usize,
    col: usize,
    value: &'a [u8],
  },
  Comment {
    line: usize,
    col: usize,
    value: &'a [u8],
  },
  Number {
    line: usize,
    col: usize,
    value: &'a [u8],
  },
  String {
    line: usize,
    col: usize,
    value: &'a [u8],
  },
  InterpolationStart {
    line: usize,
    col: usize,
  },
  InterpolationEnd {
    line: usize,
    col: usize,
  },
}

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
  pub fn new(source: &'a Vec<u8>, preserve_comments: bool) -> Tokenizer<'a> {
    let length = source.len();

    return Tokenizer {
      source: source,
      preserve_comments: preserve_comments,
      source_length: length,
    };
  }

  pub fn collect_tokens(&mut self) -> Vec<Token<'a>> {
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

        string_stack.push(index);
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

          string_stack.push(index);
          index += 1;
          continue;
        }

        if byte == b'"' {
          // Here, the " must indicate the end of a string literal section. Pop from
          // the string stack, add a new token, then advance.

          let string_start_index = string_stack.pop().unwrap();

          tokens.push(Token::String {
            line: line,
            col: string_start_index - line_start_index,
            value: &source[string_start_index + 1..index],
          });

          index += 1;
          continue;
        }

        if byte == b'$' && start_index + 1 < length && source[start_index + 1] == b'(' {
          // We must be at the beginning of an interpolation, so create a token for
          // the string literal portion leading up to the interpolation, one for the
          // interpolation start, and add to the interpolation stack.

          let string_start_index = string_stack.last().unwrap();

          tokens.push(Token::String {
            line: line,
            col: string_start_index - line_start_index,
            value: &source[string_start_index + 1..index],
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
          string_stack.push(index);

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
              value: &source[start_index..index],
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
      panic!("Unclosed interpolation");
    }

    if !string_stack.is_empty() {
      let string_start_index = string_stack.pop().unwrap();
      panic!("Unclosed string, starting at {}", string_start_index);
    }

    tokens
  }
}
