use crate::{location::Range, types::*};
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub range: Range,
	pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {
	NameNotBound { name: String },
	UnusedBinding { name: String },
	TypeMismatch { expected: Type, found: Type },
	RecursiveUnification { ty: Type },
	ParamCountMismatch { expected: usize, found: usize },
	TupleSizeMismatch { expected: usize, found: usize },
	TupleIndexNotPresent { index: usize, ty: Type },
	RecordFieldNotPresent { field: String, ty: Type },
}

impl fmt::Display for AnalysisError {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		use AnalysisErrorKind::*;

		match &self.kind {
			NameNotBound { name } => {
				write!(f, "Name `{}` is not defined.", name)
			}

			UnusedBinding { name } => write!(f, "Name `{}` is never used.", name),

			TypeMismatch { expected, found } => write!(
				f,
				"Type mismatch: expected `{}`, but found `{}`.",
				expected, found
			),

			RecursiveUnification { ty } => write!(f, "Failed to unify recursive type `{}`.", ty),

			ParamCountMismatch { expected, found } => write!(
				f,
				"Parameter count mismatch: expected {}, but found {}.",
				expected, found
			),

			TupleSizeMismatch { expected, found } => write!(
				f,
				"Tuple size mismatch: expected {} elements, but found {}.",
				expected, found
			),

			TupleIndexNotPresent { ty, index } => write!(
				f,
				"Element {} does not exist in tuple of type `{}`.",
				index, ty
			),

			RecordFieldNotPresent { ty, field } => write!(
				f,
				"Field `{}` does not exist in record of type `{}`.",
				field, ty
			),
		}
	}
}
