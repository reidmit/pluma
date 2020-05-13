use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ValueType {
  CoreString,
  CoreInt,
  CoreFloat,
  Named(String),
  Func(Vec<ValueType>, Box<ValueType>),
  Tuple(Vec<ValueType>),
  Unknown,
  Nothing,
}

impl ValueType {
  pub fn is_core_string(&self) -> bool {
    match self {
      ValueType::CoreString => true,
      _ => false,
    }
  }
}

impl fmt::Display for ValueType {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      ValueType::CoreString => write!(f, "String"),
      ValueType::CoreInt => write!(f, "Int"),
      ValueType::CoreFloat => write!(f, "Float"),
      ValueType::Named(name) => write!(f, "{}", name),
      _ => write!(f, "{:#?}", self),
    }
  }
}
