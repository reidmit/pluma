use crate::value_type::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub pos: (usize, usize),
	pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {
	CouldNotInferDefinitionType {
		name: String,
	},
	NameNotBound {
		name: String,
	},
	UnusedBinding {
		name: String,
	},
	MismatchedTypes {
		expected: ValueType,
		actual: ValueType,
	},
}

impl fmt::Display for AnalysisError {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		use AnalysisErrorKind::*;

		match &self.kind {
			CouldNotInferDefinitionType { name } => {
				write!(f, "Could not infer type for definition of '{}'.", name)
			}

			NameNotBound { name } => {
				write!(f, "Name '{}' is not defined.", name)
			}

			UnusedBinding { name } => write!(f, "Name '{}' is never used.", name),

			MismatchedTypes { expected, actual } => write!(
				f,
				"Mismatched types: expected '{}', but found '{}'.",
				expected, actual
			),
		}
	}
}
