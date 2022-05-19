use crate::ast::*;
use crate::expr_type::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub loc: (usize, usize),
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
	CalleeNotFunction {
		actual: ExprType,
	},
	IncorrectNumberOfArguments {
		arg_types: Vec<ExprType>,
		param_types: Vec<ExprType>,
	},
	MismatchedTypes {
		expected: ExprType,
		actual: ExprType,
	},
	MismatchedTypesForWhenCases {
		expected: ExprType,
		actual: ExprType,
	},
	MismatchedTypesForOperator {
		op: Operator,
		expected: ExprType,
		actual_left: ExprType,
		actual_right: ExprType,
	},
	InvalidFieldAccess,
	UndefinedFieldForType {
		field_name: String,
		receiver_type: ExprType,
	},
	PatternMismatchExpectedTuple {
		actual: ExprType,
	},
	PatternMismatchTupleSize {
		pattern_size: usize,
		subject_size: usize,
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

			CalleeNotFunction { actual } => {
				write!(f, "Cannot call value of type '{}' as a function.", actual)
			}

			IncorrectNumberOfArguments {
				arg_types,
				param_types,
			} => {
				write!(
					f,
					"Incorrect number of arguments in function call. Expected {} ({}), but got {} ({}).",
					param_types.len(),
					param_types
						.iter()
						.map(|t| format!("'{}'", t))
						.collect::<Vec<String>>()
						.join(" "),
					arg_types.len(),
					arg_types
						.iter()
						.map(|t| format!("'{}'", t))
						.collect::<Vec<String>>()
						.join(" "),
				)
			}

			UnusedBinding { name } => write!(f, "Name '{}' is never used.", name),

			MismatchedTypes { expected, actual } => write!(
				f,
				"Mismatched types: expected '{}', but found '{}'.",
				expected, actual
			),

			MismatchedTypesForWhenCases { expected, actual } => write!(
				f,
				"Mismatched types in 'when': expected all cases to have type '{}', but found '{}'.",
				expected, actual
			),

			MismatchedTypesForOperator {
				op,
				expected,
				actual_left,
				actual_right,
			} => write!(
				f,
				"Invalid types for operator '{}': expected '{}' and '{}', but found '{}' and '{}'.",
				op, expected, expected, actual_left, actual_right
			),

			InvalidFieldAccess => {
				write!(
					f,
					"Invalid field name after '.': field names can only be integers or identifiers."
				)
			}

			UndefinedFieldForType {
				field_name,
				receiver_type,
			} => {
				write!(
					f,
					"Field '{}' does not exist on type '{}'.",
					field_name, receiver_type,
				)
			}

			PatternMismatchExpectedTuple { actual } => write!(
				f,
				"Pattern mismatch: expected a tuple type, but found '{}'.",
				actual
			),

			PatternMismatchTupleSize {
				pattern_size,
				subject_size,
			} => write!(
				f,
				"Pattern mismatch: pattern expects a tuple of size {}, but found tuple of size {}.",
				pattern_size, subject_size,
			),
		}
	}
}
