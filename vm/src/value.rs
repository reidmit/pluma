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
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub enum Value {
	Nothing,
	Bool(bool),
	Int(i64),
	Float(f64),
	String(Rc<String>),
	// Bytes are an immutable, arbitrary-content byte sequence — no UTF-8
	// invariant. Distinct from String at the type level: explicit
	// `string.to-bytes` / `bytes.to-string` is the only bridge.
	Bytes(Rc<Vec<u8>>),
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
	// An immutable, insertion-ordered hash map. Keys live in `entries` in
	// the order they were first inserted; `buckets` indexes them by the
	// caller-supplied hash so lookup is O(1) average. All mutations (insert
	// / remove) return a fresh `MapData`; Rc-sharing keeps that cheap.
	Map(Rc<MapData>),
	// A mutable cell. Identity-based: two `Ref` values are equal iff they
	// point to the same underlying cell. Aliasing is intentional — passing
	// a ref to a function lets that function observe and mutate the cell.
	Ref(Rc<RefCell<Value>>),
}

pub struct MapData {
	// Insertion-ordered (key, value) pairs. Cleared and rebuilt on
	// `remove`; appended on `insert` of a new key; mutated in place when
	// replacing an existing key's value.
	pub entries: Vec<(Value, Value)>,
	// Hash → indices into `entries`. Collisions chain by walking the Vec
	// and checking `values_eq` on the keys.
	pub buckets: HashMap<i64, Vec<usize>>,
}

impl MapData {
	pub fn new() -> Self {
		Self {
			entries: Vec::new(),
			buckets: HashMap::new(),
		}
	}

	// Returns the index in `entries` of the entry whose key equals `key`
	// at hash `h`, or None if no such entry exists.
	pub fn find_index(&self, h: i64, key: &Value) -> Option<usize> {
		let chain = self.buckets.get(&h)?;
		for &idx in chain {
			if values_eq(&self.entries[idx].0, key) {
				return Some(idx);
			}
		}
		None
	}

	// Insert (or replace) without mutating self. Returns a fresh MapData.
	pub fn inserted(&self, h: i64, key: Value, value: Value) -> Self {
		let mut entries = self.entries.clone();
		let mut buckets = self.buckets.clone();
		if let Some(idx) = self.find_index(h, &key) {
			entries[idx] = (key, value);
		} else {
			let idx = entries.len();
			entries.push((key, value));
			buckets.entry(h).or_insert_with(Vec::new).push(idx);
		}
		Self { entries, buckets }
	}

	// Remove without mutating self. Returns a fresh MapData with the entry
	// gone and indices renumbered to stay dense.
	pub fn removed(&self, h: i64, key: &Value) -> Self {
		match self.find_index(h, key) {
			None => self.clone(),
			Some(removed_idx) => {
				let mut entries = Vec::with_capacity(self.entries.len() - 1);
				for (i, e) in self.entries.iter().enumerate() {
					if i != removed_idx {
						entries.push(e.clone());
					}
				}
				// Rebuild the bucket index against the renumbered entries.
				let mut buckets: HashMap<i64, Vec<usize>> = HashMap::new();
				for (h2, idxs) in &self.buckets {
					let mapped: Vec<usize> = idxs
						.iter()
						.filter_map(|&i| {
							if i == removed_idx {
								None
							} else if i > removed_idx {
								Some(i - 1)
							} else {
								Some(i)
							}
						})
						.collect();
					if !mapped.is_empty() {
						buckets.insert(*h2, mapped);
					}
				}
				Self { entries, buckets }
			}
		}
	}
}

impl Clone for MapData {
	fn clone(&self) -> Self {
		Self {
			entries: self.entries.clone(),
			buckets: self.buckets.clone(),
		}
	}
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
			Value::Bytes(b) => {
				// Render bytes in the same single-quote literal form they're
				// written in source: printable ASCII inline, everything else
				// (including '\'' and '\\') as \xNN. Round-trips to a
				// parseable bytes literal.
				write!(f, "'")?;
				for &byte in b.iter() {
					match byte {
						b'\\' => write!(f, "\\\\")?,
						b'\'' => write!(f, "\\'")?,
						0x20..=0x7e => write!(f, "{}", byte as char)?,
						_ => write!(f, "\\x{:02x}", byte)?,
					}
				}
				write!(f, "'")
			}
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
			Value::Map(m) => {
				write!(f, "{{")?;
				for (i, (k, v)) in m.entries.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}: {}", k, v)?;
				}
				write!(f, "}}")
			}
			Value::Ref(cell) => write!(f, "ref {}", cell.borrow()),
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
		(Value::Bytes(x), Value::Bytes(y)) => x == y,
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
		// Map equality is structural and order-independent: same key/value
		// set in either order is the same map. We walk one side and look
		// each key up in the other via its hash bucket.
		// Refs use reference identity, not structural equality: two cells
		// holding 5 are distinct, but a cell compared with itself is always
		// equal regardless of contents.
		(Value::Ref(a), Value::Ref(b)) => Rc::ptr_eq(a, b),
		(Value::Map(a), Value::Map(b)) => {
			if a.entries.len() != b.entries.len() {
				return false;
			}
			for (h, idxs) in &a.buckets {
				for &i in idxs {
					let (k, v) = &a.entries[i];
					match b.find_index(*h, k) {
						Some(j) => {
							if !values_eq(v, &b.entries[j].1) {
								return false;
							}
						}
						None => return false,
					}
				}
			}
			true
		}
		_ => false,
	}
}
