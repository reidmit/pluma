use std::collections::HashMap;
use std::fmt;

#[derive(Clone, PartialEq, Hash, Eq)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ExprType {
  Placeholder(usize),
  Unknown,
  Nothing,
  Bool,
  Int,
  Float,
  String,
  Regex,
  List(Box<ExprType>),
  Func(Vec<ExprType>, Box<ExprType>),
  Record(Vec<(String, ExprType)>),
  Tuple(Vec<ExprType>),
  Named(String),
  NamedWithParams(String, Vec<ExprType>),
  Invalid {
    actual: Box<ExprType>,
    expected: Box<ExprType>,
  },
}

impl ExprType {
  pub fn is_convertible_to(&self, other: &ExprType) -> bool {
    // TODO: more than just equality?
    *self == *other
  }

  pub fn has_any_placeholders(&self) -> bool {
    self.search_placeholders(None)
  }

  pub fn contains_placeholder(&self, placeholder: &usize) -> bool {
    self.search_placeholders(Some(placeholder))
  }

  fn search_placeholders(&self, placeholder: Option<&usize>) -> bool {
    match &self {
      ExprType::Placeholder(n) => match placeholder {
        Some(expected) if n == expected => true,
        None => true,
        _ => false,
      },

      ExprType::Nothing
      | ExprType::Int
      | ExprType::Float
      | ExprType::String
      | ExprType::Regex
      | ExprType::Unknown => false,

      ExprType::Func(param_types, return_type) => {
        for param_type in param_types {
          if param_type.search_placeholders(placeholder) {
            return true;
          }
        }

        return return_type.search_placeholders(placeholder);
      }

      _ => false, // TODO: ??
    }
  }

  pub fn replace_placeholders(&self, solutions: &HashMap<usize, ExprType>) -> ExprType {
    match &self {
      ExprType::Placeholder(n) if solutions.contains_key(n) => solutions.get(n).unwrap().clone(),

      ExprType::Func(param_types, return_type) => ExprType::Func(
        param_types
          .iter()
          .map(|p| p.replace_placeholders(solutions))
          .collect(),
        return_type.replace_placeholders(solutions).into(),
      ),

      other => (*other).clone(),
    }
  }
}

impl fmt::Display for ExprType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ExprType::Placeholder(n) => write!(f, "t{}", n),

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
          .map(|typ| format!("{}", typ))
          .collect::<Vec<String>>()
          .join(", ")
      ),

      ExprType::Record(entries) => write!(
        f,
        "{{{}}}",
        entries
          .iter()
          .map(|(label, typ)| format!("{}: {}", label, typ))
          .collect::<Vec<String>>()
          .join(", ")
      ),

      ExprType::Func(param_types, return_type) => {
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

      ExprType::List(element_type) => {
        write!(f, "list<{}>", element_type)
      }

      ExprType::Invalid { actual, .. } => write!(f, "{}", actual),
    }
  }
}
