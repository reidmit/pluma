use std::fmt;

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum ValueType {
  Named(String),
  Generic(String, Vec<ValueType>),
  Func(Vec<ValueType>, Box<ValueType>),
  Tuple(Vec<ValueType>),
  Constrained(TypeConstraint),
  Nothing,
  Unknown,
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum TypeConstraint {
  NamedTrait(String),
  GenericTrait(String, Vec<ValueType>),
  InlineTrait {
    fields: Vec<(String, ValueType)>,
    methods: Vec<(Vec<(String, ValueType)>, ValueType)>,
  },
}

impl ValueType {}

impl std::cmp::Eq for ValueType {}

impl fmt::Display for ValueType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ValueType::Unknown => write!(f, "unknown"),

      ValueType::Nothing => write!(f, "()"),

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

      ValueType::Tuple(entry_types) => write!(
        f,
        "({})",
        entry_types
          .iter()
          .map(|t| format!("{}", t))
          .collect::<Vec<String>>()
          .join(", ")
      ),

      ValueType::Func(param_types, return_type) => write!(
        f,
        "{{ {} -> {} }}",
        param_types
          .iter()
          .map(|t| format!("{}", t))
          .collect::<Vec<String>>()
          .join(", "),
        return_type,
      ),

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
