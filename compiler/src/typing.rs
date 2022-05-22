use std::collections::HashMap;

#[derive(Clone)]
pub enum Type {
  Var(usize),
  Int,
  String,
  Fun(Vec<Type>, Box<Type>),
}

#[derive(Clone)]
pub enum TypeScheme {
  // a monotype, e.g. int or 'a or ('a, 'b)
  Mono(Type),
  // a polytype, e.g. forall 'a 'b. ('a, 'b)
  Poly(Vec<usize>, Type),
}

pub type TypeSubstitution = HashMap<usize, Type>;

pub type TypeContext = HashMap<String, TypeScheme>;

impl std::fmt::Display for Type {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Type::Int => write!(f, "int"),

      Type::String => write!(f, "string"),

      Type::Fun(params, ret) => write!(
        f,
        "({} -> {})",
        params
          .iter()
          .map(|p| format!("{}", p))
          .collect::<Vec<String>>()
          .join(" "),
        ret
      ),

      Type::Var(var) => {
        // attempt to convert the numeric var into an ascii letter, but
        // if it's >= 26, just go with t0, t1, ...
        if *var >= 26 {
          return write!(f, "t{}", var - 26);
        }

        write!(f, "'{}", char::from_u32((*var as u32) + 97).unwrap())
      }
    }
  }
}
