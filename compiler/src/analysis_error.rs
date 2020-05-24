use pluma_ast::value_type::ValueType;
use std::fmt;

#[derive(Debug)]
pub struct AnalysisError {
  pub pos: (usize, usize),
  pub kind: AnalysisErrorKind,
}

#[derive(Debug)]
pub enum AnalysisErrorKind {
  CannotAssignToLiteral,
  UndefinedName(String),
  UndefinedMultiPartName(Vec<String>),
  UndefinedTypeConstructor(String),
  UnusedVariable(String),
  NameAlreadyInScope(String),
  CalleeNotCallable(ValueType),
  PatternMismatchExpectedTuple(ValueType),
  PatternMismatchExpectedConstructor {
    constructor_type: ValueType,
    actual_type: ValueType,
  },
  IncorrectNumberOfArguments {
    expected: usize,
    actual: usize,
  },
  PatternMismatchTupleSize {
    pattern_size: usize,
    value_size: usize,
  },
  ReassignmentTypeMismatch {
    expected: ValueType,
    actual: ValueType,
  },
  TypeMismatch {
    expected: ValueType,
    actual: ValueType,
  },
  TypeMismatchInStringInterpolation(ValueType),
}

impl fmt::Display for AnalysisError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use AnalysisErrorKind::*;

    match &self.kind {
      CannotAssignToLiteral => write!(f, "Cannot assign to a literal value."),

      NameAlreadyInScope(name) => write!(f, "Name '{}' is already defined in this scope.", name),

      UndefinedName(name) => write!(f, "Name '{}' is not defined.", name),

      UndefinedMultiPartName(names) => {
        write!(f, "Name '{}' is not defined.", names.join(" _ ") + " _")
      }

      UndefinedTypeConstructor(name) => write!(f, "Type constructor '{}' is not defined.", name),

      UnusedVariable(name) => write!(f, "Name '{}' is never used.", name),

      CalleeNotCallable(typ) => write!(f, "Cannot call value of type {} like a function.", typ),

      IncorrectNumberOfArguments { expected, actual } => write!(
        f,
        "Incorrect number of arguments given to function. Expected {}, but found {}.",
        expected, actual
      ),

      PatternMismatchExpectedTuple(typ) => write!(
        f,
        "Cannot destructure non-tuple value using a tuple pattern. Value has type {}.",
        typ
      ),

      PatternMismatchExpectedConstructor {
        constructor_type,
        actual_type,
      } => write!(
        f,
        "Cannot destructure value as a {} type. Value has type {}.",
        constructor_type, actual_type,
      ),

      PatternMismatchTupleSize {
        pattern_size,
        value_size,
      } => write!(
        f,
        "Mismatched number of elements in tuple pattern. Pattern expects {}, but value has {}.",
        pattern_size, value_size,
      ),

      TypeMismatch { expected, actual } => write!(
        f,
        "Type mismatch. Expected type {}, but found type {}.",
        expected, actual
      ),

      TypeMismatchInStringInterpolation(actual) => write!(
        f,
        "Expected type String in interpolation, but value type {}.",
        actual
      ),

      ReassignmentTypeMismatch { expected, actual } => write!(
        f,
        "Variable already has type {}, so cannot be assigned a new value of type {}.",
        expected, actual
      ),
      // _ => write!(f, "{:#?}", self.kind),
    }
  }
}
