use crate::tokens::Token;
use std::fmt;

#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ParseError {
  pub pos: (usize, usize),
  pub kind: ParseErrorKind,
}

#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ParseErrorKind {
  EmptyRegularExpression,
  EmptyRegularExpressionGroup,
  EmptyRegularExpressionCount,
  IncompleteMethodSignature,
  InvalidBinaryDigit,
  InvalidDecimalDigit,
  InvalidHexDigit,
  InvalidOctalDigit,
  InvalidRegularExpressionCountModifier,
  MissingArgumentInCall,
  MissingDefinitionBody,
  MissingDictValue,
  MissingEnumValues,
  MissingExpressionAfterDot,
  MissingExpressionAfterLabelInTuple,
  MissingExpressionAfterOperator,
  MissingExpressionAfterReturn,
  MissingIdentifier,
  MissingIndexBetweenBrackets,
  MissingLabelInTuple,
  MissingMatchCases,
  MissingQualifierAfterAs,
  MissingReturnType,
  MissingRightHandSideOfAssignment,
  MissingStructFields,
  MissingTraitConstraints,
  MissingTupleEntries,
  MissingType,
  MissingTypeInTypeAssertion,
  MissingTypeNameInTypeDefinition,
  ReturnOutsideDefinitionBody,
  UnclosedInterpolation,
  UnclosedParentheses,
  UnclosedString,
  UnexpectedDictValueInArray,
  UnexpectedEOF(Token),
  UnexpectedMethodPart,
  UnexpectedExpressionAfterDot,
  UnexpectedToken(Token),
}

impl fmt::Display for ParseError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use ParseErrorKind::*;

    match self.kind {
      EmptyRegularExpression => write!(f, "Empty regular expression."),
      EmptyRegularExpressionCount => write!(f, "Empty repetition count in regular expression."),
      EmptyRegularExpressionGroup => write!(f, "Empty grouping in regular expression."),
      IncompleteMethodSignature => write!(f, "Incomplete method signature."),
      InvalidBinaryDigit => write!(f, "Invalid binary digit."),
      InvalidDecimalDigit => write!(f, "Invalid digit."),
      InvalidHexDigit => write!(f, "Invalid hexadecimal digit."),
      InvalidOctalDigit => write!(f, "Invalid octal digit."),
      InvalidRegularExpressionCountModifier => {
        write!(f, "Invalid repetition count in regular expression.")
      }
      MissingArgumentInCall => write!(f, "Missing argument in call."),
      MissingDefinitionBody => write!(f, "Missing definition body."),
      MissingDictValue => write!(f, "Missing dictionary value."),
      MissingEnumValues => write!(f, "Missing enum values."),
      MissingExpressionAfterDot => write!(f, "Missing expression after '.'."),
      MissingExpressionAfterLabelInTuple => write!(f, "Missing value after ':' in labeled tuple."),
      MissingExpressionAfterOperator => write!(f, "Missing expression after operator."),
      MissingExpressionAfterReturn => write!(f, "Missing expression after 'return'."),
      MissingIdentifier => write!(f, "Missing identifier."),
      MissingIndexBetweenBrackets => write!(f, "Missing index between '[' and ']'."),
      MissingLabelInTuple => write!(f, "Missing identifier in labeled tuple."),
      MissingMatchCases => write!(f, "Missing cases in 'match' expression."),
      MissingQualifierAfterAs => write!(f, "Missing identifier after 'as'."),
      MissingReturnType => write!(f, "Missing return type."),
      MissingRightHandSideOfAssignment => {
        write!(f, "Missing expression after '=' in 'let' statement.")
      }
      MissingStructFields => write!(f, "Missing struct fields."),
      MissingTraitConstraints => write!(f, "Missing trait constraints."),
      MissingTupleEntries => write!(f, "Missing tuple entries."),
      MissingType => write!(f, "Missing type."),
      MissingTypeInTypeAssertion => write!(f, "Missing type in type assertion."),
      MissingTypeNameInTypeDefinition => write!(f, "Missing type name in type definition."),
      ReturnOutsideDefinitionBody => write!(f, "A 'return' cannot appear outside of a 'def'."),
      UnclosedInterpolation => write!(f, "Unterminated string interpolation. Expected a ')'."),
      UnclosedParentheses => write!(f, "Unclosed parentheses. Expected a ')'."),
      UnclosedString => write!(f, "Unterminated string. Expected a '\"'."),
      UnexpectedDictValueInArray => write!(f, "Unexpected dictionary entry in list."),
      UnexpectedEOF(expected) => write!(f, "Unexpected end of file. Expected {}.", expected),
      UnexpectedMethodPart => write!(f, "Unexpected part of method name."),
      UnexpectedExpressionAfterDot => write!(f, "Unexpected expression after '.'."),
      UnexpectedToken(expected) => write!(f, "Unexpected token. Expected {}.", expected),
    }
  }
}
