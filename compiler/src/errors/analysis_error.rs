use crate::diagnostic::Reportable;
use crate::suggest;
use crate::types::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {
	// `suggestion` carries the closest in-scope name (computed at the call
	// site, where the candidate pool is known) for the `did you mean?` help.
	NameNotBound {
		name: String,
		suggestion: Option<String>,
	},
	UnusedBinding {
		name: String,
	},
	TypeMismatch {
		expected: Type,
		found: Type,
	},
	RecursiveUnification {
		ty: Type,
	},
	ParamCountMismatch {
		expected: usize,
		found: usize,
	},
	TupleSizeMismatch {
		expected: usize,
		found: usize,
	},
	TupleIndexNotPresent {
		index: usize,
		ty: Type,
	},
	RecordFieldNotPresent {
		field: String,
		ty: Type,
	},
	EnumVariantNotPresent {
		variant: String,
		ty: Type,
		suggestion: Option<String>,
	},
	WhenNotExhaustive {
		missing: Vec<String>,
	},
	// A bare variant name was used where a qualified form is now required.
	// `suggestions` holds the ready-to-write qualified path(s) for this variant
	// — `enum.variant` for a local enum, `module.enum.variant` for an imported
	// one — so the help points exactly at what to type. (Prelude variants like
	// `some`/`ok` remain usable bare and never reach here.)
	BareVariantNeedsQualifier {
		name: String,
		suggestions: Vec<String>,
	},
	AmbiguousBareMethod {
		name: String,
		traits: Vec<String>,
	},
	DuplicateDefinition {
		name: String,
	},
	NoInstance {
		trait_name: String,
		ty: Type,
	},
	// A `wire` boundary (a value crossing as serialized bytes) required a
	// type that isn't auto-derivable. `detail` names the offending component
	// (e.g. "functions aren't serializable").
	NotWireDerivable {
		ty: Type,
		detail: String,
	},
	// A `remote def` (RPC endpoint) was declared without `public`. The client
	// can't see a private endpoint, so it could never call it — almost always
	// a mistake, and Pluma stays explicit rather than silently widening.
	RemoteDefNotPublic {
		name: String,
	},
	// A `remote def`'s signature isn't a valid RPC endpoint contract. The
	// shape must be `fun request A.. -> task R`: the first parameter is the
	// transport `request`, and the result is a `task` (the call is async).
	// `detail` says which rule was broken.
	RemoteDefSignature {
		detail: String,
	},
	UnsupportedInstanceHead {
		head: Type,
	},
	IncompleteInstance {
		trait_name: String,
		method: String,
	},
	AmbiguousTraitMethod {
		trait_name: String,
		ty: Type,
	},
	OverlappingInstance {
		trait_name: String,
		head: Type,
	},
	OrphanInstance {
		trait_name: String,
		head: Type,
	},
	RefutablePatternInLet,
	DuplicateRecordPatternField {
		field: String,
	},
	TryRhsUndetermined,
	TryUnsupportedCarrier {
		ty: Type,
	},
	TryEmptyBody,
	TryUnsupportedPattern,
	CoalesceLhsUndetermined,
	CoalesceUnsupportedCarrier {
		ty: Type,
	},
	BuiltinRequiresAnnotation,
	BuiltinMustBeTopLevelRhs,
	UnknownRegexCharacterClass {
		name: String,
	},
	WhereClauseParamNotInSignature {
		param: String,
	},
	ItemPrivate {
		name: String,
		module: String,
	},
}

// The record fields available on `ty`, if it's a record — used to suggest a
// near-miss field name and to list the real fields in a note.
fn record_fields(ty: &Type) -> Option<Vec<String>> {
	match ty {
		Type::Record(fields, _) => Some(fields.iter().map(|(name, _)| name.clone()).collect()),
		_ => None,
	}
}

fn join_names(names: &[String]) -> String {
	names
		.iter()
		.map(|n| format!("`{}`", n))
		.collect::<Vec<_>>()
		.join(", ")
}

impl fmt::Display for AnalysisError {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		use AnalysisErrorKind::*;

		match &self.kind {
			NameNotBound { name, .. } => {
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

			EnumVariantNotPresent { ty, variant, .. } => write!(
				f,
				"Variant `{}` does not exist in enum of type `{}`.",
				variant, ty
			),

			WhenNotExhaustive { missing } => {
				write!(
					f,
					"Non-exhaustive `when`: missing case for {}.",
					join_names(missing)
				)
			}

			BareVariantNeedsQualifier { name, .. } => {
				write!(f, "Variant `{}` must be qualified by its enum.", name)
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

			NotWireDerivable { ty, detail } => {
				write!(f, "Can't send `{}` across the wire: {}.", ty, detail)
			}

			RemoteDefNotPublic { name } => {
				write!(f, "A `remote def` must be `public`: `{}` is private.", name)
			}

			RemoteDefSignature { detail } => {
				write!(
					f,
					"A `remote def` must be `fun request A.. -> task R`: {}.",
					detail
				)
			}

			UnsupportedInstanceHead { head } => {
				write!(f, "Instance head `{}` is not supported.", head)
			}

			IncompleteInstance { trait_name, method } => write!(
				f,
				"Instance for trait `{}` is missing method `{}`.",
				trait_name, method
			),

			AmbiguousTraitMethod { trait_name, ty } => write!(
				f,
				"Can't resolve trait `{}` dispatch on type `{}`: the type contains unbound type variables.",
				trait_name, ty
			),

			OverlappingInstance { trait_name, head } => write!(
				f,
				"Overlapping instance: another instance of trait `{}` for head `{}` is already declared.",
				trait_name, head
			),

			OrphanInstance { trait_name, head } => write!(
				f,
				"Orphan instance: `for {} on {}` is declared outside the module that owns the trait or the type.",
				trait_name, head
			),

			RefutablePatternInLet => {
				write!(
					f,
					"This pattern can fail to match; `let` bindings must be irrefutable."
				)
			}

			DuplicateRecordPatternField { field } => write!(
				f,
				"Field `{}` is listed more than once in this record pattern.",
				field
			),

			TryRhsUndetermined => {
				write!(f, "`try`'s right-hand side has an undetermined type.")
			}

			TryUnsupportedCarrier { ty } => write!(
				f,
				"`try` only works on `option`, `result`, or `task`; this right-hand side has type `{}`.",
				ty
			),

			TryEmptyBody => write!(f, "`try` needs a continuation."),

			TryUnsupportedPattern => write!(
				f,
				"`try` only supports an identifier or `_` pattern on the left-hand side."
			),

			CoalesceLhsUndetermined => {
				write!(f, "`??`'s left-hand side has an undetermined type.")
			}

			CoalesceUnsupportedCarrier { ty } => write!(
				f,
				"`??` only works on `option` or `result`; this left-hand side has type `{}`.",
				ty
			),

			BuiltinRequiresAnnotation => {
				write!(
					f,
					"`built-in` requires a type annotation on the enclosing `def`."
				)
			}

			BuiltinMustBeTopLevelRhs => write!(
				f,
				"`built-in` may only appear as the immediate right-hand side of a top-level `def`."
			),

			UnknownRegexCharacterClass { name } => write!(
				f,
				"Unknown character class `{}` in regular expression.",
				name
			),

			WhereClauseParamNotInSignature { param } => write!(
				f,
				"`where` clause refers to type variable `{}`, which does not appear in the def's type annotation.",
				param
			),

			ItemPrivate { name, module } => {
				write!(f, "`{}` is private to module `{}`.", name, module)
			}
		}
	}
}

impl Reportable for AnalysisError {
	fn code(&self) -> &'static str {
		use AnalysisErrorKind::*;
		match &self.kind {
			NameNotBound { .. } => "E0100",
			UnusedBinding { .. } => "E0101",
			TypeMismatch { .. } => "E0102",
			RecursiveUnification { .. } => "E0103",
			ParamCountMismatch { .. } => "E0104",
			TupleSizeMismatch { .. } => "E0105",
			TupleIndexNotPresent { .. } => "E0106",
			RecordFieldNotPresent { .. } => "E0107",
			EnumVariantNotPresent { .. } => "E0108",
			WhenNotExhaustive { .. } => "E0109",
			BareVariantNeedsQualifier { .. } => "E0135",
			AmbiguousBareMethod { .. } => "E0111",
			DuplicateDefinition { .. } => "E0112",
			NoInstance { .. } => "E0113",
			NotWireDerivable { .. } => "E0114",
			RemoteDefNotPublic { .. } => "E0133",
			RemoteDefSignature { .. } => "E0134",
			UnsupportedInstanceHead { .. } => "E0115",
			IncompleteInstance { .. } => "E0116",
			AmbiguousTraitMethod { .. } => "E0117",
			OverlappingInstance { .. } => "E0118",
			OrphanInstance { .. } => "E0119",
			RefutablePatternInLet => "E0120",
			DuplicateRecordPatternField { .. } => "E0121",
			TryRhsUndetermined => "E0122",
			TryUnsupportedCarrier { .. } => "E0123",
			TryEmptyBody => "E0124",
			TryUnsupportedPattern => "E0125",
			CoalesceLhsUndetermined => "E0126",
			CoalesceUnsupportedCarrier { .. } => "E0127",
			BuiltinRequiresAnnotation => "E0128",
			BuiltinMustBeTopLevelRhs => "E0129",
			UnknownRegexCharacterClass { .. } => "E0130",
			WhereClauseParamNotInSignature { .. } => "E0131",
			ItemPrivate { .. } => "E0132",
		}
	}

	fn help(&self) -> Option<String> {
		use AnalysisErrorKind::*;
		match &self.kind {
			NameNotBound { suggestion, .. } => {
				suggestion.as_ref().map(|s| format!("did you mean `{}`?", s))
			}

			EnumVariantNotPresent { suggestion, .. } => {
				suggestion.as_ref().map(|s| format!("did you mean `{}`?", s))
			}

			BareVariantNeedsQualifier { suggestions, .. } => match suggestions.as_slice() {
				[single] => Some(format!("write `{}`.", single)),
				[first, ..] => Some(format!(
					"qualify it, e.g. `{}` (one of: {}).",
					first,
					suggestions
						.iter()
						.map(|s| format!("`{}`", s))
						.collect::<Vec<_>>()
						.join(", ")
				)),
				[] => None,
			},

			RecordFieldNotPresent { ty, field } => record_fields(ty)
				.and_then(|fields| suggest::closest(field, fields))
				.map(|s| format!("did you mean `{}`?", s)),

			WhenNotExhaustive { .. } => {
				Some("add an arm for each missing case, or a wildcard `_` arm.".to_string())
			}

			RefutablePatternInLet => Some(
				"use an identifier, `_`, tuple, or record pattern — or switch to `if`/`when` to handle the other cases."
					.to_string(),
			),

			TryRhsUndetermined => Some(
				"annotate the source so the carrier (option / result / task) can be selected."
					.to_string(),
			),

			TryEmptyBody => {
				Some("at least one expression must follow it in the surrounding block.".to_string())
			}

			TryUnsupportedPattern => {
				Some("bind to a name, then destructure with `let` on the next line.".to_string())
			}

			CoalesceLhsUndetermined => Some(
				"annotate the left-hand side so the carrier (option / result) can be selected."
					.to_string(),
			),

			BuiltinRequiresAnnotation => {
				Some("write `def name :: <type> = built-in \"tag\"`.".to_string())
			}

			UnsupportedInstanceHead { .. } => {
				Some("use a concrete type or a generic type constructor.".to_string())
			}

			AmbiguousTraitMethod { .. } => {
				Some("add a type annotation to disambiguate.".to_string())
			}

			OrphanInstance { .. } => Some(
				"declare the instance in the module that defines either the trait or the type."
					.to_string(),
			),

			ItemPrivate { module, .. } => {
				Some(format!("mark it `public` in module `{}` to use it here.", module))
			}

			_ => None,
		}
	}

	fn notes(&self) -> Vec<String> {
		use AnalysisErrorKind::*;
		match &self.kind {
			// Too-few arguments is what partial application looks like to
			// someone coming from a curried language — say so plainly.
			ParamCountMismatch { expected, found } if found < expected => vec![
				"Pluma calls are uncurried, so this isn't partial application; pass every argument, or wrap it: `fun y { f x y }`."
					.to_string(),
			],

			// `int` and `float` never mix implicitly; point at the explicit
			// conversions rather than leaving a bare type mismatch.
			TypeMismatch { expected, found }
				if matches!(
					(expected, found),
					(Type::Int, Type::Float) | (Type::Float, Type::Int)
				) =>
			{
				vec![
					"Pluma never promotes between `int` and `float` automatically; convert explicitly with `math.to-float` or `math.to-int`."
						.to_string(),
				]
			}

			RecordFieldNotPresent { ty, .. } => match record_fields(ty) {
				Some(fields) if !fields.is_empty() => {
					vec![format!("available fields: {}", join_names(&fields))]
				}
				_ => Vec::new(),
			},

			UnknownRegexCharacterClass { .. } => {
				vec!["known classes: `any`, `digit`, `letter`, `whitespace`, `word`.".to_string()]
			}

			_ => Vec::new(),
		}
	}
}
