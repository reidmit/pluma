use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub enum Token<'a> {
  EOF,
  Skipped,
  Unknown {
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
  pub is_done: bool,
  source: &'a Vec<u8>,
  preserve_comments: bool,
  source_length: usize,
  line: usize,
  line_start_index: usize,
  index: usize,
  in_interpolation: bool,
  interpolation_stack: usize,
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
      is_done: false,
      source: source,
      preserve_comments: preserve_comments,
      source_length: length,
      line: 1,
      index: 0,
      line_start_index: 0,
      in_interpolation: false,
      interpolation_stack: 0,
    };
  }

  pub fn read_tokens(&mut self) -> Token<'a> {
    if self.is_done {
      panic!("No more input to read");
    }

    let start_index = self.index;
    let length = self.source_length;

    if start_index >= length {
      self.is_done = true;
      return Token::EOF;
    }

    let line = self.line;
    let byte = self.source[start_index];

    if is_identifier_start_char(byte) {
      while self.index < length && is_identifier_char(self.source[self.index]) {
        self.index += 1;
      }

      return Token::Identifier {
        line: line,
        col: start_index - self.line_start_index,
        value: &self.source[start_index..self.index],
      };
    }

    if is_digit(byte) {
      while self.index < length && is_digit(self.source[self.index]) {
        self.index += 1;
      }

      return Token::Number {
        line: line,
        col: start_index - self.line_start_index,
        value: &self.source[start_index..self.index],
      };
    }

    return match byte {
      b' ' | b'\r' | b'\t' => {
        self.index += 1;

        Token::Skipped
      }

      b'\n' => {
        self.index += 1;
        self.line += 1;
        self.line_start_index = self.index;

        Token::Skipped
      }

      b'"' => {
        self.index += 1;

        if self.in_interpolation && self.interpolation_stack == 0 {
          self.in_interpolation = false;

          return Token::String {
            line: line,
            col: start_index - self.line_start_index,
            value: &self.source[start_index + 1..self.index],
          };
        }

        loop {
          if self.index >= length {
            break;
          }

          if self.source[self.index] == b'"' {
            let escaped = self.index > 0 && self.source[self.index - 1] == b'\\';

            if !escaped {
              break;
            }
          }

          if self.source[self.index] == b'$'
            && self.index + 1 < length
            && self.source[self.index + 1] == b'('
          {
            self.in_interpolation = true;
            break;
          }

          self.index += 1;
        }

        Token::String {
          line: line,
          col: start_index - self.line_start_index,
          value: &self.source[start_index + 1..self.index],
        }
      }

      b'$'
        if self.in_interpolation
          && self.index + 1 < length
          && self.source[self.index + 1] == b'(' =>
      {
        self.interpolation_stack += 1;
        self.index += 2;

        Token::InterpolationStart {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b'(' => {
        self.index += 1;

        Token::LeftParen {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b')' => {
        if self.in_interpolation && self.interpolation_stack == 1 {
          self.interpolation_stack -= 1;
          self.index += 1;

          return Token::InterpolationEnd {
            line: line,
            col: start_index - self.line_start_index,
          };
        }

        self.index += 1;

        Token::RightParen {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b'{' => {
        self.index += 1;

        Token::LeftBrace {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b'}' => {
        self.index += 1;

        Token::RightBrace {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b'[' => {
        self.index += 1;

        Token::LeftBracket {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b']' => {
        self.index += 1;

        Token::RightBracket {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b',' => {
        self.index += 1;

        Token::Comma {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b'.' => {
        self.index += 1;

        Token::Dot {
          line: line,
          col: start_index - self.line_start_index,
        }
      }

      b'=' => match self.source.get(self.index + 1) {
        Some(b'>') => {
          self.index += 2;

          Token::DoubleArrow {
            line: line,
            col: start_index - self.line_start_index,
          }
        }

        _ => {
          self.index += 1;

          Token::Equals {
            line: line,
            col: start_index - self.line_start_index,
          }
        }
      },

      b'-' => match self.source.get(self.index + 1) {
        Some(b'>') => {
          self.index += 2;

          Token::Arrow {
            line: line,
            col: start_index - self.line_start_index,
          }
        }

        _ => {
          self.index += 1;

          Token::Minus {
            line: line,
            col: start_index - self.line_start_index,
          }
        }
      },

      b':' => match self.source.get(self.index + 1) {
        Some(b'=') => {
          self.index += 2;

          Token::ColonEquals {
            line: line,
            col: start_index - self.line_start_index,
          }
        }

        Some(b':') => {
          self.index += 2;

          Token::DoubleColon {
            line: line,
            col: start_index - self.line_start_index,
          }
        }

        _ => {
          self.index += 1;

          Token::Colon {
            line: line,
            col: start_index - self.line_start_index,
          }
        }
      },

      b'#' => {
        while self.index < length && self.source[self.index] != b'\n' {
          self.index += 1;
        }

        if self.preserve_comments {
          Token::Comment {
            line: line,
            col: start_index - self.line_start_index,
            value: &self.source[start_index..self.index],
          }
        } else {
          Token::Skipped
        }
      }

      _ => {
        self.index += 1;

        Token::Unknown {
          line: line,
          col: start_index - self.line_start_index,
        }
      }
    };
  }
}
