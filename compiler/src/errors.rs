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
      _ => fmt::Debug::fmt(&self.kind, f),
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
  InvalidDecimalDigit,
  InvalidBinaryDigit,
  InvalidHexDigit,
  InvalidOctalDigit,
  UnclosedString,
  UnclosedInterpolation,
}
