use crate::env::Environment;
use crate::interpreter::Interpreter;
use crate::value::Value;
use compiler::ast::{LiteralKind, PatternKind, PatternNode};

// Try to match `value` against `pattern`. On success, returns true and writes
// any identifier bindings into the *current scope* of `env` (caller is
// responsible for enter_scope/leave_scope around this call). On failure,
// returns false; the env may have partial bindings written, so callers should
// match in a fresh scope and discard it on failure.
pub fn match_pattern<'ast>(
	interp: &Interpreter<'ast>,
	value: &Value<'ast>,
	pattern: &PatternNode,
	env: &mut Environment<'ast>,
) -> bool {
	match &pattern.kind {
		PatternKind::Underscore => true,

		PatternKind::Identifier(ident) => {
			// The analyzer disambiguates a bare ident in pattern position: if
			// it names a nullary variant of the subject's enum, it's a variant
			// match (not a binding). Mirror that here.
			if let Value::Variant { qualified_enum, variant, .. } = value {
				if let Some(variants) = enum_variants(interp, qualified_enum) {
					if let Some((_, arity)) = variants.iter().find(|(n, _)| n == &ident.name) {
						if *arity == 0 {
							return variant == &ident.name;
						}
					}
				}
			}
			env.define(ident.name.clone(), value.clone());
			true
		}

		PatternKind::Literal(lit) => match (&lit.kind, value) {
			(LiteralKind::Bool(b), Value::Bool(v)) => b == v,
			(LiteralKind::String(s), Value::String(v)) => s == v.as_ref(),
			(LiteralKind::FloatDecimal(f), Value::Float(v)) => f == v,
			(LiteralKind::IntDecimal(n), Value::Int(v))
			| (LiteralKind::IntHex(n), Value::Int(v))
			| (LiteralKind::IntOctal(n), Value::Int(v))
			| (LiteralKind::IntBinary(n), Value::Int(v)) => (*n as i64) == *v,
			_ => false,
		},

		PatternKind::Tuple(elems) => match value {
			Value::Tuple(values) if values.len() == elems.len() => {
				for (v, p) in values.iter().zip(elems.iter()) {
					if !match_pattern(interp, v, p, env) {
						return false;
					}
				}
				true
			}
			_ => false,
		},

		PatternKind::Record(fields) => match value {
			Value::Record(record_fields) => {
				for (field_name, field_pattern) in fields {
					match record_fields.get(&field_name.name) {
						Some(field_value) => {
							if !match_pattern(interp, field_value, field_pattern, env) {
								return false;
							}
						}
						None => return false,
					}
				}
				true
			}
			_ => false,
		},

		// `variant arg1 arg2`. The analyzer disambiguates which enum, so the
		// runtime just compares the bare variant name and the payload arity.
		PatternKind::Constructor(variant_name, sub_patterns) => match value {
			Value::Variant { variant, payload, .. } => {
				if variant != &variant_name.name {
					return false;
				}
				if payload.len() != sub_patterns.len() {
					return false;
				}
				for (v, p) in payload.iter().zip(sub_patterns.iter()) {
					if !match_pattern(interp, v, p, env) {
						return false;
					}
				}
				true
			}
			// Special case: a zero-arg constructor pattern may resolve to a
			// boolean literal at analysis time (e.g. `true`/`false` defined
			// as enum variants in a `bool` enum). The interpreter doesn't
			// see that distinction in the AST, so handle the common case of
			// `true`/`false` patterns against Bool values here.
			Value::Bool(b) if sub_patterns.is_empty() => match variant_name.name.as_str() {
				"true" => *b,
				"false" => !*b,
				_ => false,
			},
			_ => false,
		},

		// String-pattern interpolation: not implemented yet.
		PatternKind::Interpolation(_) => false,
	}
}

// Look up an enum's variants by its qualified name (e.g. `main.color`).
fn enum_variants<'a, 'ast>(
	interp: &'a Interpreter<'ast>,
	qualified_enum: &str,
) -> Option<&'a Vec<(String, usize)>> {
	let (module, enum_name) = qualified_enum.rsplit_once('.')?;
	interp.module_tops.get(module)?.enums.get(enum_name)
}
