// Value: the runtime representation of all Pluma values. Sized for compactness
// — every variant fits in 16 bytes (8-byte payload + 1-byte tag, padded to 16).
//
// Compound values (tuples, lists, records, closures, etc.) live behind `Rc`
// so cloning a Value is a refcount bump. Pluma is immutable, so the
// Rc-sharing is always safe — no copy-on-write needed.
//
// Numeric tag dispatch is the hot path in the eval loop; using a plain enum
// (vs. NaN-boxing or pointer-tagging) keeps this fast on stable Rust without
// unsafe code. See PERF-NOTES for the eventual NaN-boxing direction.

use crate::builtin::Builtin;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub enum Value {
	Nothing,
	Bool(bool),
	Int(i64),
	Float(f64),
	String(Rc<String>),
	Tuple(Rc<Vec<Value>>),
	List(Rc<Vec<Value>>),
	Record(Rc<HashMap<String, Value>>),
	Variant(Rc<VariantData>),
	Closure(Rc<ClosureData>),
	Builtin(Builtin),
	Regex(Rc<RegexData>),
	VariantCtor(Rc<VariantCtorData>),
	// A typeclass dictionary: a positional array of method values, indexed by
	// trait declaration order. Built per-instance at program load (concrete
	// instances) or per-call (parametric instances; phase 3). The VM never
	// inspects a Dict directly — only `GetDictField` reads from one.
	Dict(Rc<Vec<Value>>),
}

pub struct VariantData {
	pub qualified_enum: Rc<String>,
	pub variant: Rc<String>,
	pub payload: Vec<Value>,
}

pub struct ClosureData {
	pub fn_idx: usize,
	// Rc-shared so cloning a closure (which happens on every Call) is just a
	// refcount bump rather than a fresh Vec allocation.
	pub captures: Rc<Vec<Value>>,
}

pub struct VariantCtorData {
	pub qualified_enum: Rc<String>,
	pub variant: Rc<String>,
	pub arity: usize,
}

pub struct RegexData {
	pub compiled: regex::Regex,
}

// Display drives `to-string`. Stays consistent with REFERENCE.md spellings.
impl std::fmt::Display for Value {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Value::Int(n) => write!(f, "{}", n),
			Value::Float(n) => {
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
			Value::Variant(v) => {
				let bare = v
					.qualified_enum
					.rsplit_once('.')
					.map(|(_, n)| n)
					.unwrap_or(&v.qualified_enum);
				write!(f, "{}.{}", bare, v.variant)?;
				for arg in &v.payload {
					write!(f, " {}", arg)?;
				}
				Ok(())
			}
			Value::Closure(_) => write!(f, "<closure>"),
			Value::Builtin(_) => write!(f, "<builtin>"),
			Value::Regex(r) => write!(f, "<regex {}>", r.compiled.as_str()),
			Value::VariantCtor(c) => {
				let bare = c
					.qualified_enum
					.rsplit_once('.')
					.map(|(_, n)| n)
					.unwrap_or(&c.qualified_enum);
				write!(f, "<ctor {}.{}>", bare, c.variant)
			}
			Value::Dict(_) => write!(f, "<dict>"),
		}
	}
}

// Structural equality for `==` / `!=` / `contains`. Type system enforces same
// type on both sides, so we only need to compare like with like. Closures,
// builtins, ctors, and regexes always compare false.
pub fn values_eq(a: &Value, b: &Value) -> bool {
	match (a, b) {
		(Value::Int(x), Value::Int(y)) => x == y,
		(Value::Float(x), Value::Float(y)) => x == y,
		(Value::Bool(x), Value::Bool(y)) => x == y,
		(Value::String(x), Value::String(y)) => x == y,
		(Value::Nothing, Value::Nothing) => true,
		(Value::Tuple(xs), Value::Tuple(ys)) => {
			xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(a, b)| values_eq(a, b))
		}
		(Value::List(xs), Value::List(ys)) => {
			xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(a, b)| values_eq(a, b))
		}
		(Value::Record(xs), Value::Record(ys)) => {
			xs.len() == ys.len()
				&& xs
					.iter()
					.all(|(k, v)| ys.get(k).map_or(false, |yv| values_eq(v, yv)))
		}
		(Value::Variant(a), Value::Variant(b)) => {
			a.qualified_enum == b.qualified_enum
				&& a.variant == b.variant
				&& a.payload.len() == b.payload.len()
				&& a
					.payload
					.iter()
					.zip(b.payload.iter())
					.all(|(a, b)| values_eq(a, b))
		}
		_ => false,
	}
}
