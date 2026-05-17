use crate::env::Environment;
use compiler::ast::ExprNode;
use std::collections::HashMap;
use std::rc::Rc;

// Runtime values produced by the interpreter. The 'ast lifetime ties closures
// and regex values to the AST owned by the Compiler — the interpreter never
// outlives a compile.
pub enum Value<'ast> {
	Int(i64),
	Float(f64),
	String(Rc<String>),
	Bool(bool),
	Nothing,
	Tuple(Vec<Value<'ast>>),
	List(Rc<Vec<Value<'ast>>>),
	Record(Rc<HashMap<String, Value<'ast>>>),
	Variant {
		// Fully-qualified `<module>.<enum>` name.
		qualified_enum: String,
		variant: String,
		payload: Vec<Value<'ast>>,
	},
	Closure {
		params: Vec<String>,
		body: &'ast [ExprNode],
		env: Environment<'ast>,
		// Module the `fun {...}` literal appeared in. When the closure is
		// called from another module, the body still resolves free identifiers
		// against this module's top-level defs.
		defining_module: String,
	},
	// Built-in functions live in eval/builtin.rs.
	Builtin(Builtin),
	// Constructor for an enum variant with payload. When applied to its args,
	// produces a Value::Variant. Zero-payload variants are materialized
	// directly as Value::Variant — this is only used for variants with args.
	VariantCtor {
		qualified_enum: String,
		variant: String,
		arity: usize,
	},
	Regex(Rc<regex::Regex>),
	// Opaque — no regex operations are exposed yet, so we just hold the AST
	// node. When operations land we'll compile it.
}

#[derive(Clone, Copy)]
pub enum Builtin {
	Print,
	ToString,
	Matches,
	ListLength,
	ListIsEmpty,
	ListReverse,
	ListConcat,
	ListContains,
	ListMap,
	ListFilter,
	ListFold,
	ListEach,
}

impl<'ast> Clone for Value<'ast> {
	fn clone(&self) -> Self {
		match self {
			Value::Int(n) => Value::Int(*n),
			Value::Float(n) => Value::Float(*n),
			Value::String(s) => Value::String(s.clone()),
			Value::Bool(b) => Value::Bool(*b),
			Value::Nothing => Value::Nothing,
			Value::Tuple(elems) => Value::Tuple(elems.clone()),
			Value::List(elems) => Value::List(elems.clone()),
			Value::Record(fields) => Value::Record(fields.clone()),
			Value::Variant {
				qualified_enum,
				variant,
				payload,
			} => Value::Variant {
				qualified_enum: qualified_enum.clone(),
				variant: variant.clone(),
				payload: payload.clone(),
			},
			Value::Closure {
				params,
				body,
				env,
				defining_module,
			} => Value::Closure {
				params: params.clone(),
				body,
				env: env.clone(),
				defining_module: defining_module.clone(),
			},
			Value::Builtin(b) => Value::Builtin(*b),
			Value::Regex(r) => Value::Regex(r.clone()),
			Value::VariantCtor {
				qualified_enum,
				variant,
				arity,
			} => Value::VariantCtor {
				qualified_enum: qualified_enum.clone(),
				variant: variant.clone(),
				arity: *arity,
			},
		}
	}
}

// Display drives `to-string`. Stays consistent with REFERENCE.md spellings:
// `()` for nothing, `enum.variant args...` for variants, `{k: v, k: v}` for
// records, `[a, b, c]` for lists, `(a, b)` for tuples.
impl<'ast> std::fmt::Display for Value<'ast> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Value::Int(n) => write!(f, "{}", n),
			Value::Float(n) => {
				// Always show at least one decimal so floats are visually
				// distinct from ints.
				if n.fract() == 0.0 && n.is_finite() {
					write!(f, "{:.1}", n)
				} else {
					write!(f, "{}", n)
				}
			}
			Value::String(s) => write!(f, "{}", s),
			Value::Bool(b) => write!(f, "{}", b),
			Value::Nothing => write!(f, "()"),
			Value::Tuple(elems) => {
				write!(f, "(")?;
				for (i, v) in elems.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}", v)?;
				}
				write!(f, ")")
			}
			Value::List(elems) => {
				write!(f, "[")?;
				for (i, v) in elems.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}", v)?;
				}
				write!(f, "]")
			}
			Value::Record(fields) => {
				write!(f, "{{")?;
				let mut entries: Vec<_> = fields.iter().collect();
				entries.sort_by(|a, b| a.0.cmp(b.0));
				for (i, (k, v)) in entries.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}: {}", k, v)?;
				}
				write!(f, "}}")
			}
			Value::Variant {
				qualified_enum,
				variant,
				payload,
			} => {
				let bare = qualified_enum.rsplit_once('.').map(|(_, n)| n).unwrap_or(qualified_enum);
				write!(f, "{}.{}", bare, variant)?;
				for arg in payload {
					write!(f, " {}", arg)?;
				}
				Ok(())
			}
			Value::Closure { .. } => write!(f, "<closure>"),
			Value::Builtin(_) => write!(f, "<builtin>"),
			Value::Regex(r) => write!(f, "<regex {}>", r.as_str()),
			Value::VariantCtor { qualified_enum, variant, .. } => {
				let bare = qualified_enum.rsplit_once('.').map(|(_, n)| n).unwrap_or(qualified_enum);
				write!(f, "<ctor {}.{}>", bare, variant)
			}
		}
	}
}
