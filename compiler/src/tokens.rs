// TODO: use this to only derive in tests
// #[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Debug, PartialEq)]
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

  DecimalDigits {
    line: usize,
    col: usize,
    value: &'a [u8],
  },

  BinaryDigits {
    line: usize,
    col: usize,
    value: &'a [u8],
  },

  OctalDigits {
    line: usize,
    col: usize,
    value: &'a [u8],
  },

  HexDigits {
    line: usize,
    col: usize,
    value: &'a [u8],
  },

  String {
    line_start: usize,
    col_start: usize,
    line_end: usize,
    col_end: usize,
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
