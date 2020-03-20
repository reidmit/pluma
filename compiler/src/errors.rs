use std::fmt;

#[derive(Debug, Copy, Clone)]
pub struct ParseError {
  pub pos: (usize, usize),
  pub kind: ParseErrorKind,
}

#[derive(Debug, Copy, Clone)]
pub enum ParseErrorKind {
  FailedToReadFile,
  InvalidBinaryDigit,
  InvalidDecimalDigit,
  InvalidHexDigit,
  InvalidOctalDigit,
  MissingDefinitionBody,
  MissingDictValue,
  MissingEnumValues,
  MissingExpressionAfterDot,
  MissingExpressionAfterOperator,
  MissingExpressionAfterReturn,
  MissingIdentifier,
  MissingIndexBetweenBrackets,
  MissingMatchCases,
  MissingQualifierAfterAs,
  MissingReturnType,
  MissingRightHandSideOfAssignment,
  MissingStructFields,
  MissingType,
  ReturnOutsideDefinitionBody,
  UnclosedInterpolation,
  UnclosedParentheses,
  UnclosedString,
  UnexpectedDictValueInArray,
  UnexpectedEOF,
  UnexpectedToken,
}

impl fmt::Display for ParseError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use ParseErrorKind::*;

    match self.kind {
      UnexpectedToken => write!(f, "Unexpected token"),
      MissingDefinitionBody => write!(f, "Missing definition body"),
      MissingRightHandSideOfAssignment => {
        write!(f, "Missing expression after '=' in 'let' statement.")
      }
      _ => write!(f, "{:?}", self.kind),
    }
  }
}
