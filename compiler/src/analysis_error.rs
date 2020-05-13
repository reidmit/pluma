use crate::types::ValueType;
use std::fmt;

#[derive(Debug)]
pub struct AnalysisError {
  pub pos: (usize, usize),
  pub kind: AnalysisErrorKind,
}

#[derive(Debug)]
pub enum AnalysisErrorKind {
  UndefinedVariable(String),
  UnusedVariable(String),
  NameAlreadyInScope(String),
  ReassignmentTypeMismatch {
    expected: ValueType,
    actual: ValueType,
  },
  TypeMismatch {
    expected: ValueType,
    actual: ValueType,
  },
}

impl fmt::Display for AnalysisError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use AnalysisErrorKind::*;

    match &self.kind {
      NameAlreadyInScope(name) => write!(f, "Name '{}' is already defined in this scope.", name),
      UndefinedVariable(name) => write!(f, "Name '{}' is not defined.", name),
      UnusedVariable(name) => write!(f, "Name '{}' is never used.", name),
      TypeMismatch { expected, actual } => write!(
        f,
        "Type mismatch. Expected type {}, but found type {}.",
        expected, actual
      ),
      ReassignmentTypeMismatch { expected, actual } => write!(
        f,
        "Variable already has type {}, so cannot be assigned a new value of type {}.",
        expected, actual
      ),
    }
  }
}
