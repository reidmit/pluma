// TODO: use this to only derive in tests
// #[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Debug, PartialEq)]
pub enum Token<'a> {
  Arrow { start: usize, end: usize },
  BinaryDigits { start: usize, end: usize, value: &'a [u8] },
  Colon { start: usize, end: usize },
  ColonEquals { start: usize, end: usize },
  Comma { start: usize, end: usize },
  Comment { start: usize, end: usize, value: &'a [u8] },
  DecimalDigits { start: usize, end: usize, value: &'a [u8] },
  Dot { start: usize, end: usize },
  DoubleArrow { start: usize, end: usize },
  DoubleColon { start: usize, end: usize },
  Equals { start: usize, end: usize },
  HexDigits { start: usize, end: usize, value: &'a [u8] },
  Identifier { start: usize, end: usize, value: &'a [u8] },
  InterpolationEnd { start: usize, end: usize },
  InterpolationStart { start: usize, end: usize },
  LeftBrace { start: usize, end: usize },
  LeftBracket { start: usize, end: usize },
  LeftParen { start: usize, end: usize },
  LineBreak { start: usize, end: usize },
  Minus { start: usize, end: usize },
  OctalDigits { start: usize, end: usize, value: &'a [u8] },
  Pipe { start: usize, end: usize },
  RightBrace { start: usize, end: usize },
  RightBracket { start: usize, end: usize },
  RightParen { start: usize, end: usize },
  StringLiteral { start: usize, end: usize, value: &'a [u8] },
  Unexpected { start: usize, end: usize },
}

pub fn get_token_location(token: &Token) -> (usize, usize) {
  match token {
    Token::Arrow { start, end } => (*start, *end),
    Token::BinaryDigits { start, end, .. } => (*start, *end),
    Token::Colon { start, end } => (*start, *end),
    Token::ColonEquals { start, end } => (*start, *end),
    Token::Comma { start, end } => (*start, *end),
    Token::Comment { start, end, .. } => (*start, *end),
    Token::DecimalDigits { start, end, .. } => (*start, *end),
    Token::Dot { start, end } => (*start, *end),
    Token::DoubleArrow { start, end } => (*start, *end),
    Token::DoubleColon { start, end } => (*start, *end),
    Token::Equals { start, end } => (*start, *end),
    Token::HexDigits { start, end, .. } => (*start, *end),
    Token::Identifier { start, end, .. } => (*start, *end),
    Token::InterpolationEnd { start, end } => (*start, *end),
    Token::InterpolationStart { start, end } => (*start, *end),
    Token::LeftBrace { start, end } => (*start, *end),
    Token::LeftBracket { start, end } => (*start, *end),
    Token::LeftParen { start, end } => (*start, *end),
    Token::LineBreak { start, end } => (*start, *end),
    Token::Minus { start, end } => (*start, *end),
    Token::OctalDigits { start, end, .. } => (*start, *end),
    Token::Pipe { start, end, .. } => (*start, *end),
    Token::RightBrace { start, end } => (*start, *end),
    Token::RightBracket { start, end } => (*start, *end),
    Token::RightParen { start, end } => (*start, *end),
    Token::StringLiteral { start, end, .. } => (*start, *end),
    Token::Unexpected { start, end } => (*start, *end),
  }
}