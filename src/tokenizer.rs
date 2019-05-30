use std::collections::VecDeque;

#[derive(Debug, Clone)]
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
    let mut tokens = Vec::new();

    let source = self.source;
    let length = self.source_length;

    let mut index = 0;
    let mut line = 0;
    let mut line_start_index = 0;
    let mut string_stack = 0;
    let mut interpolation_stack = 0;
    let mut string_literal_start_index = 0;

    /*
      oh "hello $(name) wow"

      - string starts
      - interpolation starts, string ends
    */

    while index < length {
      let start_index = index;
      let byte = source[start_index];

      if string_stack == 0 && byte == b'"' {
        string_stack += 1;
        string_literal_start_index = index;
        index += 1;
        continue;
      }

      if string_stack > 0 {
        if byte == b'"' {
          string_stack -= 1;
          index += 1;

          tokens.push(Token::String {
            line: line,
            col: string_literal_start_index - line_start_index,
            value: &source[string_literal_start_index..index],
          });

          continue;
        } else if byte == b'$' && start_index + 1 < length && source[start_index + 1] == b'(' {
          index += 2;
          interpolation_stack += 1;
          continue;
        } else if byte == b')' {
          index += 1;
          interpolation_stack -= 1;
          continue;
        }

        index += 1;
        continue;
      }

      if is_identifier_start_char(byte) {
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

      if is_digit(byte) {
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

        _ => {
          index += 1;

          tokens.push(Token::Unexpected {
            line: line,
            col: start_index - line_start_index,
          })
        }
      };
    }

    tokens
  }
}
