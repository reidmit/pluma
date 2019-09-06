// TODO: use this to only derive in tests
// #[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Debug, PartialEq)]
pub enum Token<'a> {
  Unexpected {
    start: usize,
    end: usize,
  },

  LeftParen {
    start: usize,
    end: usize,
  },

  RightParen {
    start: usize,
    end: usize,
  },

  LeftBrace {
    start: usize,
    end: usize,
  },

  RightBrace {
    start: usize,
    end: usize,
  },

  LeftBracket {
    start: usize,
    end: usize,
  },

  RightBracket {
    start: usize,
    end: usize,
  },

  Comma {
    start: usize,
    end: usize,
  },

  Dot {
    start: usize,
    end: usize,
  },

  Colon {
    start: usize,
    end: usize,
  },

  Equals {
    start: usize,
    end: usize,
  },

  Minus {
    start: usize,
    end: usize,
  },

  Arrow {
    start: usize,
    end: usize,
  },

  DoubleArrow {
    start: usize,
    end: usize,
  },

  DoubleColon {
    start: usize,
    end: usize,
  },

  ColonEquals {
    start: usize,
    end: usize,
  },

  Identifier {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  Comment {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  DecimalDigits {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  BinaryDigits {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  OctalDigits {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  HexDigits {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  String {
    start: usize,
    end: usize,
    value: &'a [u8],
  },

  InterpolationStart {
    start: usize,
    end: usize,
  },

  InterpolationEnd {
    start: usize,
    end: usize,
  },

  LineBreak {
    start: usize,
    end: usize,
  },
}
