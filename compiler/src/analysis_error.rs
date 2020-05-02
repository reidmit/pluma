use crate::types::Type;
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
  TypeMismatch { expected: Type, actual: Type },
}

impl fmt::Display for AnalysisError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use AnalysisErrorKind::*;

    match &self.kind {
      UndefinedVariable(name) => write!(f, "Name '{}' is not defined.", name),
      UnusedVariable(name) => write!(f, "Name '{}' is never used.", name),
      TypeMismatch { expected, actual } => write!(
        f,
        "Type mismatch. Expected type {}, but found type {}.",
        expected, actual
      ),
    }
  }
}
