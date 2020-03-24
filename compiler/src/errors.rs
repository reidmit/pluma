use crate::tokens::Token;
use std::fmt;

#[derive(Debug, Copy, Clone)]
pub struct ParseError {
  pub pos: (usize, usize),
  pub kind: ParseErrorKind,
}

#[derive(Debug, Copy, Clone)]
pub enum ParseErrorKind {
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
  MissingTypeNameInTypeDefinition,
  ReturnOutsideDefinitionBody,
  UnclosedInterpolation,
  UnclosedParentheses,
  UnclosedString,
  UnexpectedDictValueInArray,
  UnexpectedEOF(Token),
  UnexpectedToken(Token),
}

impl fmt::Display for ParseError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use ParseErrorKind::*;

    match self.kind {
      InvalidBinaryDigit => write!(f, "Invalid binary digit."),
      InvalidDecimalDigit => write!(f, "Invalid digit."),
      InvalidHexDigit => write!(f, "Invalid hexadecimal digit."),
      InvalidOctalDigit => write!(f, "Invalid octal digit."),
      MissingDefinitionBody => write!(f, "Missing definition body."),
      MissingRightHandSideOfAssignment => {
        write!(f, "Missing expression after '=' in 'let' statement.")
      }
      UnclosedInterpolation => write!(f, "Unterminated string interpolation. Expected a ')'."),
      UnclosedParentheses => write!(f, "Unclosed parentheses. Expected a ')'."),
      UnclosedString => write!(f, "Unterminated string. Expected a '\"'."),
      UnexpectedEOF(expected) => write!(f, "Unexpected end of file. Expected {}.", expected),
      UnexpectedToken(expected) => write!(f, "Unexpected token. Expected {}.", expected),
      _ => write!(f, "{:#?}", self.kind),
    }
  }
}

#[derive(Debug, Clone)]
pub struct ImportError {
  pub kind: ImportErrorKind,
}

#[derive(Debug, Clone)]
pub enum ImportErrorKind {
  CyclicalDependency(Vec<String>),
  ModuleNotFound(String, String),
}

impl fmt::Display for ImportError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use ImportErrorKind::*;

    match &self.kind {
      CyclicalDependency(module_names) => write!(
        f,
        "Import cycle between modules: '{}'",
        module_names.join("' -> '")
      ),
      ModuleNotFound(module_name, module_path) => write!(
        f,
        "Module '{}' not found.\n\nExpected to find it at '{}'.",
        module_name, module_path,
      ),
    }
  }
}
