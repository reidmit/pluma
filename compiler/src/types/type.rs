use std::collections::HashSet;

#[derive(Clone)]
pub enum Type {
	Unknown,
	Var(usize),
	Bool,
	Int,
	Float,
	Regex,
	// An opaque point on the wall clock (UTC), backed by an i64 nanosecond
	// count since the Unix epoch. Produced and consumed only by `core.time`
	// builtins; the surface language can't peek at the raw count.
	Instant,
	// An opaque signed time span, backed by an i64 nanosecond count. Also
	// owned by `core.time`.
	Duration,
	String,
	Bytes,
	Nothing,
	Tuple(Vec<Type>),
	PartialTuple(usize, Box<Type>),
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
	// `core.dict`, not on the type itself.
	Dict(Box<Type>, Box<Type>),
	// `Ref(inner)`. A mutable cell holding a value of type `inner`. Created
	// via `ref.new`, read/written through `core.ref` operations. Equality
	// on refs is reference identity, not structural.
	Ref(Box<Type>),
}

impl Type {
	pub fn contains_var(&self, var: usize) -> bool {
		match &self {
			Type::Var(n) => var == *n,

			Type::Nothing
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Bytes
			| Type::Regex
			| Type::Instant
			| Type::Duration
			| Type::Unknown => false,

			Type::PartialTuple(_, element_type) => element_type.contains_var(var),

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
			| Type::Regex
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

			Type::PartialTuple(_, element_type) => {
				vars.extend(element_type.free_vars());
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
			| Type::Regex
			| Type::Instant
			| Type::Duration
			| Type::String
			| Type::Bytes
			| Type::Nothing
			| Type::Var(_) => {}

			Type::PartialTuple(_, element_type) => {
				vars.extend(element_type.free_row_vars());
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

impl std::fmt::Display for Type {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
			Type::Regex => write!(f, "regex"),
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

			Type::PartialTuple(index, element) => {
				write!(f, "({}: {}, ...)", index, element)
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

			Type::Var(var) => {
				// return write!(f, "'t{}", var); // temporary, i think

				// attempt to convert the numeric var into an ascii letter, but
				// if it's >= 26, just go with t0, t1, ...
				if *var >= 26 {
					return write!(f, "'t{}", var - 26);
				}

				write!(f, "'{}", char::from_u32((*var as u32) + 97).unwrap())
			}
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Type {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self)
	}
}
