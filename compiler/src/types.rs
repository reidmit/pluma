use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Type {
  CoreString,
  CoreInt,
  CoreFloat,
  Named(Uuid),
  Func(Vec<Type>, Box<Type>),
  Tuple(Vec<Type>),
  Unknown,
  Nothing,
}

impl Type {
  pub fn is_core_string(&self) -> bool {
    match self {
      Type::CoreString => true,
      _ => false,
    }
  }
}

impl fmt::Display for Type {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    match self {
      Type::CoreString => write!(f, "String"),
      Type::CoreInt => write!(f, "Int"),
      Type::CoreFloat => write!(f, "Float"),
      _ => write!(f, "{:#?}", self),
    }
  }
}
