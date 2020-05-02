use std::fmt;

#[derive(Debug, Clone)]
pub struct AnalysisError {
  pub pos: (usize, usize),
  pub kind: AnalysisErrorKind,
}

#[derive(Debug, Clone)]
pub enum AnalysisErrorKind {
  UndefinedVariable(String),
  UnusedVariable(String),
}

impl fmt::Display for AnalysisError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use AnalysisErrorKind::*;

    match &self.kind {
      UndefinedVariable(name) => write!(f, "Name '{}' is not defined.", name),
      UnusedVariable(name) => write!(f, "Variable '{}' is never used.", name),
    }
  }
}
