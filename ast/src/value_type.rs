use std::fmt;

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum ValueType {
  Named(String),
  Generic(String, Vec<ValueType>),
  Func(Vec<ValueType>, Box<ValueType>),
  Tuple(Vec<ValueType>),
  Nothing,
  Unknown,
}

impl ValueType {}

impl std::cmp::Eq for ValueType {}

impl fmt::Display for ValueType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ValueType::Unknown => write!(f, "?"),

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
        "{} -> {}",
        param_types
          .iter()
          .map(|t| format!("{}", t))
          .collect::<Vec<String>>()
          .join(", "),
        return_type,
      ),
    }
  }
}
