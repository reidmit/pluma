use crate::types::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
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
	EnumVariantNotPresent { variant: String, ty: Type },
	WhenNotExhaustive { missing: Vec<String> },
	AmbiguousVariant { name: String, enums: Vec<String> },
	AmbiguousBareMethod { name: String, traits: Vec<String> },
	DuplicateDefinition { name: String },
	NoInstance { trait_name: String, ty: Type },
	UnsupportedInstanceHead { head: Type },
	IncompleteInstance { trait_name: String, method: String },
	AmbiguousTraitMethod { trait_name: String, ty: Type },
	OverlappingInstance { trait_name: String, head: Type },
	OrphanInstance { trait_name: String, head: Type },
	RefutablePatternInLet,
	DuplicateRecordPatternField { field: String },
	TryRhsUndetermined,
	TryUnsupportedCarrier { ty: Type },
	TryEmptyBody,
	TryUnsupportedPattern,
	BuiltinRequiresAnnotation,
	BuiltinMustBeTopLevelRhs,
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

			EnumVariantNotPresent { ty, variant } => write!(
				f,
				"Variant `{}` does not exist in enum of type `{}`.",
				variant, ty
			),

			WhenNotExhaustive { missing } => {
				let formatted = missing
					.iter()
					.map(|n| format!("`{}`", n))
					.collect::<Vec<_>>()
					.join(", ");
				write!(f, "Non-exhaustive `when`: missing case for {}.", formatted)
			}

			AmbiguousVariant { name, enums } => {
				let formatted = enums
					.iter()
					.map(|n| format!("`{}`", n))
					.collect::<Vec<_>>()
					.join(" or ");
				write!(
					f,
					"Variant `{}` is ambiguous: it could refer to {}.",
					name, formatted
				)
			}

			AmbiguousBareMethod { name, traits } => {
				let formatted = traits
					.iter()
					.map(|t| format!("`{}.{}`", t, name))
					.collect::<Vec<_>>()
					.join(" or ");
				write!(
					f,
					"Method `{}` is ambiguous: qualify it as {}.",
					name, formatted
				)
			}

			DuplicateDefinition { name } => write!(f, "Duplicate top-level definition `{}`.", name),

			NoInstance { trait_name, ty } => write!(
				f,
				"No instance of trait `{}` for type `{}`.",
				trait_name, ty
			),

			UnsupportedInstanceHead { head } => write!(
				f,
				"Instance head `{}` is not supported. Use a concrete type or a generic type constructor.",
				head
			),

			IncompleteInstance { trait_name, method } => write!(
				f,
				"Instance for trait `{}` is missing method `{}`.",
				trait_name, method
			),

			AmbiguousTraitMethod { trait_name, ty } => write!(
				f,
				"Cannot resolve trait `{}` dispatch on type `{}`: the type contains unbound type variables. Add a type annotation to disambiguate.",
				trait_name, ty
			),

			OverlappingInstance { trait_name, head } => write!(
				f,
				"Overlapping instance: another instance of trait `{}` for head `{}` is already declared.",
				trait_name, head
			),

			OrphanInstance { trait_name, head } => write!(
				f,
				"Orphan instance: `for {} on {}` must be declared in the module that defines either the trait or the type.",
				trait_name, head
			),

			RefutablePatternInLet => write!(
				f,
				"This pattern can fail to match. `let` bindings require an irrefutable pattern (identifier, wildcard, tuple, or record). Use `if` or `when` to handle the cases."
			),

			DuplicateRecordPatternField { field } => write!(
				f,
				"Field `{}` is listed more than once in this record pattern.",
				field
			),

			TryRhsUndetermined => write!(
				f,
				"`try`'s right-hand side has an undetermined type. Add a type annotation to its source so the carrier (option / result / task) can be selected."
			),

			TryUnsupportedCarrier { ty } => write!(
				f,
				"`try` only works on `option`, `result`, or `task`; this right-hand side has type `{}`.",
				ty
			),

			TryEmptyBody => write!(
				f,
				"`try` needs a continuation — at least one expression must follow it in the surrounding block."
			),

			TryUnsupportedPattern => write!(
				f,
				"`try` currently only supports an identifier or `_` pattern on the left-hand side. Bind to a name and destructure with `let` on the next line."
			),

			BuiltinRequiresAnnotation => write!(
				f,
				"`built-in` requires a type annotation on the enclosing `def` (`def name :: <type> = built-in \"tag\"`)."
			),

			BuiltinMustBeTopLevelRhs => write!(
				f,
				"`built-in` may only appear as the immediate right-hand side of a top-level `def`."
			),
		}
	}
}
