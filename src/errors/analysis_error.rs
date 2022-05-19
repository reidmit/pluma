use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub span: (usize, usize),
	pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {
	NameNotBound { name: String },
	UnusedBinding { name: String },
}

impl fmt::Display for AnalysisError {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		use AnalysisErrorKind::*;

		match &self.kind {
			NameNotBound { name } => {
				write!(f, "Name '{}' is not defined.", name)
			}

			UnusedBinding { name } => write!(f, "Name '{}' is never used.", name),
		}
	}
}
