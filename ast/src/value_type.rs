use std::fmt;

#[derive(Clone, PartialEq, Hash)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ValueType {
	Int,
	Float,
	String,
	Named(String),
	Generic(String, Vec<ValueType>),
	Func(Box<ValueType>, Box<ValueType>),
	Tuple(Vec<(Option<String>, ValueType)>),
	Constrained(TypeConstraint),
	Nothing,
	Unknown,
}

#[derive(Clone, PartialEq, Hash)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeConstraint {
	NamedTrait(String),
	GenericTrait(String, Vec<ValueType>),
	InlineTrait {
		fields: Vec<(String, ValueType)>,
		methods: Vec<(Vec<(String, ValueType)>, ValueType)>,
	},
}

impl ValueType {
	pub fn func_param_type(&self) -> &ValueType {
		match &self {
			ValueType::Func(param_type, _) => param_type,
			_ => unreachable!(),
		}
	}

	pub fn func_return_type(&self) -> ValueType {
		match &self {
			ValueType::Func(_, return_type) => *return_type.clone(),
			_ => unreachable!(),
		}
	}
}

impl std::cmp::Eq for ValueType {}

impl fmt::Display for ValueType {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		match self {
			ValueType::Unknown => write!(f, "unknown"),

			ValueType::Nothing => write!(f, "()"),

			ValueType::Int => write!(f, "Int"),

			ValueType::Float => write!(f, "Float"),

			ValueType::String => write!(f, "String"),

			ValueType::Named(name) => write!(f, "{}", name),

			ValueType::Generic(name, generic_params) => write!(
				f,
				"{}<{}>",
				name,
				generic_params
					.iter()
					.map(|p| format!("{}", p))
					.collect::<Vec<String>>()
					.join(", ")
			),

			ValueType::Tuple(entries) => write!(
				f,
				"({})",
				entries
					.iter()
					.map(|(label, typ)| {
						match label {
							Some(label) => format!("{}: {}", label, typ),
							None => format!("{}", typ),
						}
					})
					.collect::<Vec<String>>()
					.join(", ")
			),

			ValueType::Func(param_type, return_type) => {
				write!(f, "{{ {} -> {} }}", param_type, return_type,)
			}

			ValueType::Constrained(constraint) => match constraint {
				TypeConstraint::NamedTrait(name) => write!(f, "{}", name),

				TypeConstraint::GenericTrait(name, generic_params) => write!(
					f,
					"{}<{}>",
					name,
					generic_params
						.iter()
						.map(|p| format!("{}", p))
						.collect::<Vec<String>>()
						.join(", ")
				),

				TypeConstraint::InlineTrait { fields, methods } => {
					write!(f, "(")?;

					for (field_name, field_type) in fields {
						write!(f, ". {} :: {}, ", field_name, field_type)?;
					}

					for (method_parts, return_type) in methods {
						write!(f, ". ")?;

						for (part_name, part_param_type) in method_parts {
							write!(f, "{} {} ", part_name, part_param_type)?;
						}

						write!(f, "-> {}, ", return_type)?;
					}

					write!(f, ")")?;

					Ok(())
				}
			},
		}
	}
}
