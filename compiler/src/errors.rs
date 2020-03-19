use std::fmt;

#[derive(Debug, Copy, Clone)]
pub struct ParseError {
  pub pos: (usize, usize),
  pub kind: ParseErrorKind,
}

impl fmt::Display for ParseError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self.kind {
      ParseErrorKind::UnexpectedToken => write!(f, "Unexpected token"),
      ParseErrorKind::MissingDefinitionBody => write!(f, "Missing definition body"),
      ParseErrorKind::MissingRightHandSideOfAssignment => {
        write!(f, "Missing expression after '=' in 'let' statement.")
      }
      _ => fmt::Debug::fmt(self, f),
    }
  }
}

#[derive(Debug, Copy, Clone)]
pub enum ParseErrorKind {
  FailedToReadFile,
  UnexpectedDictValueInArray,
  UnexpectedEOF,
  UnexpectedToken,
  UnclosedParentheses,
  MissingIdentifier,
  MissingIndexBetweenBrackets,
  MissingDefinitionBody,
  MissingDictValue,
  MissingEnumValues,
  MissingExpressionAfterDot,
  MissingExpressionAfterOperator,
  MissingExpressionAfterReturn,
  MissingMatchCases,
  MissingQualifierAfterAs,
  MissingReturnType,
  MissingRightHandSideOfAssignment,
  MissingStructFields,
  MissingType,
  ReturnOutsideDefinitionBody,
}

#[derive(Debug)]
pub enum TokenizeError {
  InvalidDecimalDigit(usize, usize),
  InvalidBinaryDigit(usize, usize),
  InvalidHexDigit(usize, usize),
  InvalidOctalDigit(usize, usize),
  UnclosedString(usize, usize),
  UnclosedInterpolation(usize, usize),
}

impl TokenizeError {
  pub fn pos(&self) -> (usize, usize) {
    match self {
      TokenizeError::InvalidDecimalDigit(start, end) => (*start, *end),
      TokenizeError::InvalidBinaryDigit(start, end) => (*start, *end),
      TokenizeError::InvalidHexDigit(start, end) => (*start, *end),
      TokenizeError::InvalidOctalDigit(start, end) => (*start, *end),
      TokenizeError::UnclosedString(start, end) => (*start, *end),
      TokenizeError::UnclosedInterpolation(start, end) => (*start, *end),
    }
  }
}

impl fmt::Display for TokenizeError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      _ => fmt::Debug::fmt(self, f),
    }
  }
}
