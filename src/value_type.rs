use std::fmt;

#[derive(Clone, PartialEq, Hash)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ValueType {
  Int,
  Float,
  String,
  Named(String),
  Generic(String, Vec<ValueType>),
  Func(Vec<ValueType>, Box<ValueType>),
  Tuple(Vec<(Option<String>, ValueType)>),
  Nothing,
  Unknown,
}

impl fmt::Display for ValueType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ValueType::Unknown => write!(f, "unknown"),

      ValueType::Nothing => write!(f, "()"),

      ValueType::Int => write!(f, "int"),

      ValueType::Float => write!(f, "float"),

      ValueType::String => write!(f, "string"),

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

      ValueType::Func(param_types, return_type) => {
        write!(
          f,
          "{} -> {}",
          param_types
            .iter()
            .map(|p| format!("{}", p))
            .collect::<Vec<String>>()
            .join(", "),
          return_type
        )
      }
    }
  }
}
