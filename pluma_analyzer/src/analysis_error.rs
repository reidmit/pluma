use pluma_ast::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
  pub pos: (usize, usize),
  pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {
  CannotAssignToLiteral,
  UndefinedName(String),
  UndefinedType(ValueType),
  UndefinedTypeInMethodDef(ValueType),
  UndefinedMultiPartName(Vec<String>),
  UndefinedTypeConstructor(String),
  UndefinedFieldForType {
    field_name: String,
    receiver_type: ValueType,
  },
  UndefinedMethodForType {
    method_name_parts: Vec<String>,
    receiver_type: ValueType,
  },
  UnusedVariable(String),
  NameAlreadyInScope(String),
  CalleeNotCallable(ValueType),
  PatternMismatchUnknownField {
    field_name: String,
    value_type: ValueType,
  },
  PatternMismatchExpectedTuple(ValueType),
  PatternMismatchExpectedConstructor {
    constructor_type: ValueType,
    actual_type: ValueType,
  },
  IncorrectNumberOfArguments {
    expected: usize,
    actual: usize,
  },
  ParamCountMismatchInDefinition {
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
  ReturnTypeMismatch {
    expected: ValueType,
    actual: ValueType,
  },
  ParameterTypeMismatch {
    expected: ValueType,
    actual: ValueType,
  },
  TypeMismatchInTypeAssertion {
    expected: ValueType,
    actual: ValueType,
  },
  TypeMismatchInStringInterpolation(ValueType),
  TypeMismatchInMatchCase {
    expected: ValueType,
    actual: ValueType,
  },
}

impl fmt::Display for AnalysisError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use AnalysisErrorKind::*;

    match &self.kind {
      CannotAssignToLiteral => write!(f, "Cannot assign to a literal value."),

      NameAlreadyInScope(name) => write!(f, "Name '{}' is already defined in this scope.", name),

      UndefinedName(name) => write!(f, "Name '{}' is not defined.", name),

      UndefinedType(typ) => write!(f, "Type {} is not defined.", typ),

      UndefinedTypeInMethodDef(typ) => write!(f, "Cannot define method on undefined type {}.", typ),

      UndefinedMultiPartName(names) => {
        write!(f, "Name '{}' is not defined.", names.join(" _ ") + " _")
      }

      UndefinedTypeConstructor(name) => write!(f, "Type constructor '{}' is not defined.", name),

      UndefinedFieldForType {
        field_name,
        receiver_type,
      } => write!(
        f,
        "Field '{}' does not exist on type {}.",
        field_name, receiver_type
      ),

      UndefinedMethodForType {
        method_name_parts,
        receiver_type,
      } => write!(
        f,
        "Method '{}' is not defined for type {}.",
        method_name_parts.join(" "),
        receiver_type
      ),

      UnusedVariable(name) => write!(f, "Name '{}' is never used.", name),

      CalleeNotCallable(typ) => write!(f, "Cannot call value of type {} like a function.", typ),

      IncorrectNumberOfArguments { expected, actual } => write!(
        f,
        "Incorrect number of arguments given to function. Expected {}, but found {}.",
        expected, actual
      ),

      ParamCountMismatchInDefinition { expected, actual } => write!(
        f,
        "Incorrect number of parameters in function body. The signature shows {}, but found {}.",
        expected, actual
      ),

      PatternMismatchUnknownField {
        field_name,
        value_type,
      } => write!(
        f,
        "Cannot destructure unknown field '{}' in pattern. Value has type {}.",
        field_name, value_type,
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

      ParameterTypeMismatch { expected, actual } => write!(
        f,
        "Parameter type mismatch. Expected type {}, but found type {}.",
        expected, actual
      ),

      ReturnTypeMismatch { expected, actual } => write!(
        f,
        "Return type mismatch. Expected type {}, but found type {}.",
        expected, actual
      ),

      TypeMismatchInStringInterpolation(actual) => write!(
        f,
        "Expected type String in interpolation, but found type {}.",
        actual
      ),

      TypeMismatchInTypeAssertion { expected, actual } => write!(
        f,
        "Type assertion failed. Type {} is not convertible to type {}.",
        actual, expected,
      ),

      TypeMismatchInMatchCase { expected, actual } => write!(
        f,
        "Expected type {} for this case, but found type {}.",
        expected, actual,
      ),

      ReassignmentTypeMismatch { expected, actual } => write!(
        f,
        "Variable already has type {}, so cannot be assigned a new value of type {}.",
        expected, actual
      ),

      InvalidValueForConst => write!(f, "Value of const statement must be a literal."),
    }
  }
}
