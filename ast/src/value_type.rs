use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ValueType {
  Named(String),
  Func(Vec<ValueType>, Box<ValueType>),
  Tuple(Vec<ValueType>),
  Nothing,
  Unknown,
}

impl ValueType {}

impl fmt::Display for ValueType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ValueType::Named(name) => write!(f, "{}", name),
      ValueType::Tuple(entry_types) => write!(
        f,
        "({})",
        entry_types
          .iter()
          .map(|t| format!("{}", t))
          .collect::<Vec<String>>()
          .join(", ")
      ),
      _ => write!(f, "{:#?}", self),
    }
  }
}
