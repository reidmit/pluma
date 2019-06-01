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
  pub fn from_source(source: &'a Vec<u8>) -> Tokenizer<'a> {
    let length = source.len();

    return Tokenizer {
      source: source,
      preserve_comments: true,
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn empty() {
    let src = "";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(tokens, vec![])
  }

  #[test]
  fn identifer_tokens() {
    let src = "hello world";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(
      tokens,
      vec![
        Token::Identifier {
          line: 0,
          col: 0,
          value: "hello".as_bytes()
        },
        Token::Identifier {
          line: 0,
          col: 6,
          value: "world".as_bytes()
        },
      ]
    )
  }

  #[test]
  fn number_tokens() {
    let src = "hello 1 47 wow";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(
      tokens,
      vec![
        Token::Identifier {
          line: 0,
          col: 0,
          value: "hello".as_bytes()
        },
        Token::Number {
          line: 0,
          col: 6,
          value: "1".as_bytes()
        },
        Token::Number {
          line: 0,
          col: 8,
          value: "47".as_bytes()
        },
        Token::Identifier {
          line: 0,
          col: 11,
          value: "wow".as_bytes()
        },
      ]
    )
  }

  #[test]
  fn symbol_tokens() {
    let src = "{ . } ( , ) : [ :: ] := = => ->";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(tokens.len(), 14);
    assert_eq!(tokens[0], Token::LeftBrace { line: 0, col: 0 });
    assert_eq!(tokens[1], Token::Dot { line: 0, col: 2 });
    assert_eq!(tokens[2], Token::RightBrace { line: 0, col: 4 });
    assert_eq!(tokens[3], Token::LeftParen { line: 0, col: 6 });
    assert_eq!(tokens[4], Token::Comma { line: 0, col: 8 });
    assert_eq!(tokens[5], Token::RightParen { line: 0, col: 10 });
    assert_eq!(tokens[6], Token::Colon { line: 0, col: 12 });
    assert_eq!(tokens[7], Token::LeftBracket { line: 0, col: 14 });
    assert_eq!(tokens[8], Token::DoubleColon { line: 0, col: 16 });
    assert_eq!(tokens[9], Token::RightBracket { line: 0, col: 19 });
    assert_eq!(tokens[10], Token::ColonEquals { line: 0, col: 21 });
    assert_eq!(tokens[11], Token::Equals { line: 0, col: 24 });
    assert_eq!(tokens[12], Token::DoubleArrow { line: 0, col: 26 });
    assert_eq!(tokens[13], Token::Arrow { line: 0, col: 29 });
  }

  #[test]
  fn unexpected_tokens() {
    let src = "(@$@)";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(tokens.len(), 5);
    assert_eq!(tokens[0], Token::LeftParen { line: 0, col: 0 });
    assert_eq!(tokens[1], Token::Unexpected { line: 0, col: 1 });
    assert_eq!(tokens[2], Token::Unexpected { line: 0, col: 2 });
    assert_eq!(tokens[3], Token::Unexpected { line: 0, col: 3 });
    assert_eq!(tokens[4], Token::RightParen { line: 0, col: 4 });
  }

  #[test]
  fn strings_without_interpolations() {
    let src = "\"hello\" \"\" \"world\"";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(tokens.len(), 3);
    assert_eq!(
      tokens[0],
      Token::String {
        line: 0,
        col: 0,
        value: "hello".as_bytes()
      }
    );
    assert_eq!(
      tokens[1],
      Token::String {
        line: 0,
        col: 8,
        value: "".as_bytes()
      }
    );
    assert_eq!(
      tokens[2],
      Token::String {
        line: 0,
        col: 11,
        value: "world".as_bytes()
      }
    );
  }

  #[test]
  fn strings_with_interpolations() {
    let src = "\"hello $(name)!\" nice \"$(str)\"";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(tokens.len(), 11);

    assert_eq!(
      tokens[0],
      Token::String {
        line: 0,
        col: 0,
        value: "hello ".as_bytes()
      }
    );

    assert_eq!(tokens[1], Token::InterpolationStart { line: 0, col: 7 });

    assert_eq!(
      tokens[2],
      Token::Identifier {
        line: 0,
        col: 9,
        value: "name".as_bytes()
      }
    );

    assert_eq!(tokens[3], Token::InterpolationEnd { line: 0, col: 13 });

    assert_eq!(
      tokens[4],
      Token::String {
        line: 0,
        col: 13,
        value: "!".as_bytes()
      }
    );

    assert_eq!(
      tokens[5],
      Token::Identifier {
        line: 0,
        col: 17,
        value: "nice".as_bytes()
      }
    );

    assert_eq!(
      tokens[6],
      Token::String {
        line: 0,
        col: 22,
        value: "".as_bytes()
      }
    );

    assert_eq!(tokens[7], Token::InterpolationStart { line: 0, col: 23 });

    assert_eq!(
      tokens[8],
      Token::Identifier {
        line: 0,
        col: 25,
        value: "str".as_bytes()
      }
    );

    assert_eq!(tokens[9], Token::InterpolationEnd { line: 0, col: 28 });

    assert_eq!(
      tokens[10],
      Token::String {
        line: 0,
        col: 28,
        value: "".as_bytes()
      }
    );
  }

  #[test]
  fn strings_with_nested_interpolations() {
    let src = "\"hello $(name \"inner $(o)\" wow)!\"";
    let v = Vec::from(src);
    let tokens = Tokenizer::from_source(&v).collect_tokens();

    assert_eq!(tokens.len(), 11);

    assert_eq!(
      tokens[0],
      Token::String {
        line: 0,
        col: 0,
        value: "hello ".as_bytes()
      }
    );

    assert_eq!(tokens[1], Token::InterpolationStart { line: 0, col: 7 });

    assert_eq!(
      tokens[2],
      Token::Identifier {
        line: 0,
        col: 9,
        value: "name".as_bytes()
      }
    );

    assert_eq!(
      tokens[3],
      Token::String {
        line: 0,
        col: 14,
        value: "inner ".as_bytes()
      }
    );

    assert_eq!(tokens[4], Token::InterpolationStart { line: 0, col: 21 });

    assert_eq!(
      tokens[5],
      Token::Identifier {
        line: 0,
        col: 23,
        value: "o".as_bytes()
      }
    );

    assert_eq!(tokens[6], Token::InterpolationEnd { line: 0, col: 24 });

    assert_eq!(
      tokens[7],
      Token::String {
        line: 0,
        col: 24,
        value: "".as_bytes()
      }
    );

    assert_eq!(
      tokens[8],
      Token::Identifier {
        line: 0,
        col: 27,
        value: "wow".as_bytes()
      }
    );

    assert_eq!(tokens[9], Token::InterpolationEnd { line: 0, col: 30 });

    assert_eq!(
      tokens[10],
      Token::String {
        line: 0,
        col: 30,
        value: "!".as_bytes()
      }
    );
  }
}
