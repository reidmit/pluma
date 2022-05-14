use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub pos: (usize, usize),
	pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {
	CouldNotInferDefinitionType { name: String },
	UnusedVariable(String),
}

impl fmt::Display for AnalysisError {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		use AnalysisErrorKind::*;

		match &self.kind {
			CouldNotInferDefinitionType { name } => {
				write!(f, "Could not infer type for definition of '{}'.", name)
			}
			UnusedVariable(name) => write!(f, "Variable '{}' is never used.", name),
			_ => Ok(()),
		}
	}
}
