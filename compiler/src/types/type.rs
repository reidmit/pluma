use std::collections::HashSet;

#[derive(Clone)]
pub enum Type {
	Unknown,
	Var(usize),
	Bool,
	Int,
	Float,
	// An opaque point on the wall clock (UTC), backed by an i64 nanosecond
	// count since the Unix epoch. Produced and consumed only by `std/time`
	// builtins; the surface language can't peek at the raw count.
	Instant,
	// An opaque signed time span, backed by an i64 nanosecond count. Also
	// owned by `std/time`.
	Duration,
	String,
	Bytes,
	Nothing,
	Tuple(Vec<Type>),
	// `PartialTuple(fields, tail)` — the tuple analogue of an open `Record`.
	// `fields` is a set of known `(index, type)` pairs; `tail = Some(rid)` is a
	// row variable standing in for the indices we haven't pinned down yet,
	// while `tail = None` is closed (exactly these indices). Element access
	// (`e.0`) produces an open partial tuple with a single known index, and
	// accessing several indices on the same value merges them through the row
	// variable — exactly how `Record` accumulates fields. Unifying against a
	// concrete `Tuple` closes the tail.
	PartialTuple(Vec<(usize, Type)>, Option<usize>),
	// `Record(fields, tail)`. `tail = None` is a closed record (exactly these
	// fields). `tail = Some(rid)` is an open record with row variable `rid`
	// standing in for whatever additional fields the subject may carry. Field
	// access, `{a, ...}` patterns, and `{a, ...rest}` patterns all produce
	// open records — row polymorphism lets us track "has at least these"
	// without picking specific extras.
	Record(Vec<(String, Type)>, Option<usize>),
	Fun(Vec<Type>, Box<Type>),
	// `Enum(qualified-name, type-args)`. `type-args` is empty for monomorphic
	// enums and matches the enum's declared param arity for generic ones.
	// Unification matches on both: names must agree AND args unify pairwise.
	Enum(String, Vec<Type>),
	List(Box<Type>),
	// `Dict(key, value)`. A hash-backed associative table; keys must have a
	// `hash` instance, but that constraint lives on the operations in
	// `std/dict`, not on the type itself.
	Dict(Box<Type>, Box<Type>),
	// `Ref(inner)`. A mutable cell holding a value of type `inner`. Created
	// via `ref.new`, read/written through `std/ref` operations. Equality
	// on refs is reference identity, not structural.
	Ref(Box<Type>),
}

impl Type {
	// Smart constructor for a partial tuple that collapses the fully-known case
	// back to a concrete `Tuple`. A partial tuple is "fully known" when it's
	// closed (no row tail) and its indices cover a gap-free `0..n` range — at
	// that point it's just an ordinary tuple, so resolved types read as `(a, b)`
	// rather than `(0: a, 1: b)`. Anything still open, or with gaps/duplicate
	// indices, stays a `PartialTuple`. Used by the substitution/resolution paths
	// (`apply_to_type`, `deep_resolve`); the unifier's tuple arms accept either
	// shape coming back.
	pub fn partial_tuple(mut fields: Vec<(usize, Type)>, tail: Option<usize>) -> Type {
		if tail.is_none() && !fields.is_empty() {
			fields.sort_by_key(|(i, _)| *i);
			if fields.iter().enumerate().all(|(i, (idx, _))| i == *idx) {
				return Type::Tuple(fields.into_iter().map(|(_, t)| t).collect());
			}
		}
		Type::PartialTuple(fields, tail)
	}

	pub fn contains_var(&self, var: usize) -> bool {
		match &self {
			Type::Var(n) => var == *n,

			Type::Nothing
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Bytes
			| Type::Instant
			| Type::Duration
			| Type::Unknown => false,

			Type::PartialTuple(field_types, _tail) => {
				for (_, field_type) in field_types {
					if field_type.contains_var(var) {
						return true;
					}
				}

				false
			}

			Type::Enum(_, args) => args.iter().any(|t| t.contains_var(var)),

			Type::List(element_type) => element_type.contains_var(var),

			Type::Dict(key_type, value_type) => {
				key_type.contains_var(var) || value_type.contains_var(var)
			}

			Type::Ref(inner_type) => inner_type.contains_var(var),

			Type::Tuple(element_types) => {
				for element_type in element_types {
					if element_type.contains_var(var) {
						return true;
					}
				}

				false
			}

			Type::Record(field_types, _tail) => {
				for (_, field_type) in field_types {
					if field_type.contains_var(var) {
						return true;
					}
				}

				false
			}

			Type::Fun(param_types, return_type) => {
				for param_type in param_types {
					if param_type.contains_var(var) {
						return true;
					}
				}

				return_type.contains_var(var)
			}
		}
	}

	pub fn free_vars(&self) -> HashSet<usize> {
		let mut vars = HashSet::new();

		match self {
			Type::Unknown
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::Instant
			| Type::Duration
			| Type::String
			| Type::Bytes
			| Type::Nothing => {
				// no vars to add
			}

			Type::Var(n) => {
				vars.insert(*n);
			}

			Type::PartialTuple(field_types, _tail) => {
				for (_, field_type) in field_types {
					vars.extend(field_type.free_vars());
				}
			}

			Type::Enum(_, args) => {
				for arg in args {
					vars.extend(arg.free_vars());
				}
			}

			Type::List(element_type) => {
				vars.extend(element_type.free_vars());
			}

			Type::Dict(key_type, value_type) => {
				vars.extend(key_type.free_vars());
				vars.extend(value_type.free_vars());
			}

			Type::Ref(inner_type) => {
				vars.extend(inner_type.free_vars());
			}

			Type::Tuple(element_types) => {
				for element_type in element_types {
					vars.extend(element_type.free_vars());
				}
			}

			Type::Record(field_types, _tail) => {
				for (_, field_type) in field_types {
					vars.extend(field_type.free_vars());
				}
			}

			Type::Fun(param_types, return_type) => {
				for param_type in param_types {
					vars.extend(param_type.free_vars())
				}

				vars.extend(return_type.free_vars())
			}
		}

		vars
	}

	// Row variables that appear in this type — distinct from `free_vars`,
	// which tracks type variables. Generalization quantifies over both
	// kinds; the unifier substitutes them through separate maps.
	pub fn free_row_vars(&self) -> HashSet<usize> {
		let mut vars = HashSet::new();
		match self {
			Type::Unknown
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::Instant
			| Type::Duration
			| Type::String
			| Type::Bytes
			| Type::Nothing
			| Type::Var(_) => {}

			Type::PartialTuple(field_types, tail) => {
				for (_, field_type) in field_types {
					vars.extend(field_type.free_row_vars());
				}
				if let Some(rid) = tail {
					vars.insert(*rid);
				}
			}

			Type::Enum(_, args) => {
				for arg in args {
					vars.extend(arg.free_row_vars());
				}
			}

			Type::List(element_type) => {
				vars.extend(element_type.free_row_vars());
			}

			Type::Dict(key_type, value_type) => {
				vars.extend(key_type.free_row_vars());
				vars.extend(value_type.free_row_vars());
			}

			Type::Ref(inner_type) => {
				vars.extend(inner_type.free_row_vars());
			}

			Type::Tuple(element_types) => {
				for element_type in element_types {
					vars.extend(element_type.free_row_vars());
				}
			}

			Type::Record(field_types, tail) => {
				for (_, field_type) in field_types {
					vars.extend(field_type.free_row_vars());
				}
				if let Some(rid) = tail {
					vars.insert(*rid);
				}
			}

			Type::Fun(param_types, return_type) => {
				for param_type in param_types {
					vars.extend(param_type.free_row_vars());
				}
				vars.extend(return_type.free_row_vars());
			}
		}
		vars
	}
}

// Type-variable display is canonicalized per top-level signature: the first
// distinct `Type::Var` encountered in a rendering becomes `a`, the next `b`,
// and so on, regardless of its internal numeric id. This keeps signatures
// readable (`a -> a` instead of `t102 -> t102`) and stable across analyzer
// changes that merely shift the fresh-var counter, so snapshots don't churn.
//
// The mapping lives in a thread-local that resets whenever the outermost
// `Display::fmt` for a `Type` begins. Recursion (and the `format!` inside
// `maybe_add_parens`) re-enters `fmt` at depth > 0, so a whole type tree shares
// one numbering. A `Drop` guard restores the depth even on the early `return`s
// in the match below.
thread_local! {
	static VAR_DISPLAY: std::cell::RefCell<VarDisplayState> =
		std::cell::RefCell::new(VarDisplayState::default());
}

#[derive(Default)]
struct VarDisplayState {
	depth: usize,
	names: std::collections::HashMap<usize, usize>,
	next: usize,
}

struct VarDisplayGuard;

impl VarDisplayGuard {
	fn enter() -> Self {
		VAR_DISPLAY.with(|state| {
			let mut state = state.borrow_mut();
			if state.depth == 0 {
				state.names.clear();
				state.next = 0;
			}
			state.depth += 1;
		});
		VarDisplayGuard
	}
}

impl Drop for VarDisplayGuard {
	fn drop(&mut self) {
		VAR_DISPLAY.with(|state| state.borrow_mut().depth -= 1);
	}
}

fn display_var_name(var: usize) -> String {
	let index = VAR_DISPLAY.with(|state| {
		let mut state = state.borrow_mut();
		match state.names.get(&var) {
			Some(index) => *index,
			None => {
				let index = state.next;
				state.next += 1;
				state.names.insert(var, index);
				index
			}
		}
	});

	// 0..=25 -> a..z, then a1, b1, ... for the rare signature with >26 vars.
	// Bare letters (no leading apostrophe) so a displayed type reads exactly as
	// you'd write it in source — `fun (list a) -> a`, not ML's `'a`.
	let letter = char::from_u32((index % 26) as u32 + 97).unwrap();
	let suffix = index / 26;
	if suffix == 0 {
		letter.to_string()
	} else {
		format!("{}{}", letter, suffix)
	}
}

impl std::fmt::Display for Type {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let _var_display_guard = VarDisplayGuard::enter();

		let maybe_add_parens = |t: &Type| {
			let s = format!("{}", t);
			if s.contains(" ") {
				format!("({})", s)
			} else {
				s
			}
		};

		match self {
			Type::Unknown => write!(f, "?"),
			Type::Bool => write!(f, "bool"),
			Type::Int => write!(f, "int"),
			Type::Float => write!(f, "float"),
			Type::String => write!(f, "string"),
			Type::Bytes => write!(f, "bytes"),
			Type::Instant => write!(f, "instant"),
			Type::Duration => write!(f, "duration"),
			Type::Nothing => write!(f, "nothing"),

			Type::Enum(name, args) => {
				// Internally enum names are fully-qualified
				// (`<defining-module>.<enum-name>`). For display, show just
				// the bare enum name, with space-separated type args
				// (matching `list int` style).
				let bare = name.rsplit_once('.').map(|(_, n)| n).unwrap_or(name);
				if args.is_empty() {
					write!(f, "{}", bare)
				} else {
					write!(
						f,
						"{} {}",
						bare,
						args
							.iter()
							.map(maybe_add_parens)
							.collect::<Vec<String>>()
							.join(" "),
					)
				}
			}

			Type::Fun(params, ret) => write!(
				f,
				"{} -> {}",
				params
					.iter()
					.map(maybe_add_parens)
					.collect::<Vec<String>>()
					.join(" "),
				ret
			),

			Type::PartialTuple(fields, tail) => {
				// Render positionally, like a tuple: fill indices we never learned
				// with `_` placeholders and mark an open tail with a trailing
				// `...`. So `.0` + `.1` reads `(a, b, ...)`, `.0` + `.2` reads
				// `(a, _, b, ...)`, and `.2` alone `(_, _, b, ...)`.
				if fields.is_empty() {
					return match tail {
						None => write!(f, "()"),
						Some(_) => write!(f, "(...)"),
					};
				}
				let max_index = fields.iter().map(|(i, _)| *i).max().unwrap();
				let mut slots: Vec<String> = vec!["_".to_string(); max_index + 1];
				for (index, element) in fields {
					slots[*index] = maybe_add_parens(element);
				}
				let body = slots.join(", ");
				match tail {
					None => write!(f, "({})", body),
					Some(_rid) => write!(f, "({}, ...)", body),
				}
			}

			Type::Tuple(elements) => write!(
				f,
				"({})",
				elements
					.iter()
					.map(maybe_add_parens)
					.collect::<Vec<String>>()
					.join(", "),
			),

			Type::Record(fields, tail) => {
				// Sort fields alphabetically for stable display. Substitution
				// can merge fields from different sources in an order that
				// depends on solve order, which would otherwise make
				// diagnostics non-deterministic.
				let mut sorted: Vec<&(String, Type)> = fields.iter().collect();
				sorted.sort_by(|a, b| a.0.cmp(&b.0));
				let field_str = sorted
					.iter()
					.map(|(field_name, field_type)| format!("{}: {}", field_name, field_type))
					.collect::<Vec<String>>()
					.join(", ");
				match tail {
					None => write!(f, "{{{}}}", field_str),
					Some(_rid) => {
						// Row var id is internal state — most types only have
						// one row var visible at a time, so the bare `...`
						// reads better than `...ρ7`. Diagnostics that need to
						// distinguish multiple rows in the same type can
						// upgrade this later.
						if fields.is_empty() {
							write!(f, "{{...}}")
						} else {
							write!(f, "{{{}, ...}}", field_str)
						}
					}
				}
			}

			Type::List(element_type) => write!(f, "list {}", maybe_add_parens(element_type)),

			Type::Dict(key_type, value_type) => write!(
				f,
				"dict {} {}",
				maybe_add_parens(key_type),
				maybe_add_parens(value_type),
			),

			Type::Ref(inner_type) => write!(f, "ref {}", maybe_add_parens(inner_type)),

			Type::Var(var) => write!(f, "{}", display_var_name(*var)),
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Type {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self)
	}
}
