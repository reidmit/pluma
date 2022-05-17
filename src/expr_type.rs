use std::fmt;

#[derive(Clone, PartialEq, Hash)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ExprType {
  Unknown,
  Nothing,
  Bool,
  Int,
  Float,
  String,
  Regex,
  List(Box<ExprType>),
  Dict(Box<ExprType>, Box<ExprType>),
  Func(Vec<ExprType>, Box<ExprType>),
  Tuple(Vec<(Option<String>, ExprType)>),
  Named(String),
  NamedWithParams(String, Vec<ExprType>),
}

impl ExprType {
  pub fn is_convertible_to(&self, other: &ExprType) -> bool {
    // TODO: more than just equality?
    *self == *other
  }
}

impl fmt::Display for ExprType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ExprType::Unknown => write!(f, "unknown"),

      ExprType::Nothing => write!(f, "nothing"),

      ExprType::Bool => write!(f, "bool"),

      ExprType::Int => write!(f, "int"),

      ExprType::Float => write!(f, "float"),

      ExprType::String => write!(f, "string"),

      ExprType::Regex => write!(f, "regex"),

      ExprType::Named(name) => write!(f, "{}", name),

      ExprType::NamedWithParams(name, params) => {
        write!(
          f,
          "{}<{}>",
          name,
          params
            .iter()
            .map(|p| format!("{}", p))
            .collect::<Vec<String>>()
            .join(", ")
        )
      }

      ExprType::Tuple(entries) => write!(
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

      ExprType::Func(param_types, return_type) => {
        write!(
          f,
          "fn {} -> {}",
          param_types
            .iter()
            .map(|p| format!("{}", p))
            .collect::<Vec<String>>()
            .join(", "),
          return_type
        )
      }

      ExprType::List(element_type) => {
        write!(f, "list<{}>", element_type)
      }

      ExprType::Dict(key_type, val_type) => {
        write!(f, "dict<{}, {}>", key_type, val_type)
      }
    }
  }
}
