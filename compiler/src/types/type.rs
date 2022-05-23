use std::collections::HashSet;

#[derive(Clone)]
pub enum Type {
  Unknown,
  Var(usize),
  Bool,
  Int,
  Float,
  Regex,
  String,
  Nothing,
  Tuple(Vec<Type>),
  PartialTuple(usize, Box<Type>),
  Fun(Vec<Type>, Box<Type>),
}

impl Type {
  pub fn contains_var(&self, var: usize) -> bool {
    match &self {
      Type::Var(n) => var == *n,

      Type::Nothing
      | Type::Bool
      | Type::Int
      | Type::Float
      | Type::String
      | Type::Regex
      | Type::Unknown => false,

      Type::PartialTuple(_, element_type) => element_type.contains_var(var),

      Type::Tuple(element_types) => {
        for element_types in element_types {
          if element_types.contains_var(var) {
            return true;
          }
        }

        false
      }

      Type::Fun(param_types, return_type) => {
        for param_type in param_types {
          if param_type.contains_var(var) {
            return true;
          }
        }

        return_type.contains_var(var)
      }
    }
  }

  pub fn free_vars(&self) -> HashSet<usize> {
    let mut vars = HashSet::new();

    match self {
      Type::Unknown
      | Type::Bool
      | Type::Int
      | Type::Float
      | Type::Regex
      | Type::String
      | Type::Nothing => {
        // no vars to add
      }

      Type::Var(n) => {
        vars.insert(*n);
      }

      Type::PartialTuple(_, element_type) => {
        vars.extend(element_type.free_vars());
      }

      Type::Tuple(element_types) => {
        for element_type in element_types {
          vars.extend(element_type.free_vars());
        }
      }

      Type::Fun(param_types, return_type) => {
        for param_type in param_types {
          vars.extend(param_type.free_vars())
        }

        vars.extend(return_type.free_vars())
      }
    }

    vars
  }
}

impl std::fmt::Display for Type {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let maybe_add_parens = |t: &Type| {
      let s = format!("{}", t);
      if s.contains(" ") {
        format!("({})", s)
      } else {
        s
      }
    };

    match self {
      Type::Unknown => write!(f, "?"),
      Type::Bool => write!(f, "bool"),
      Type::Int => write!(f, "int"),
      Type::Float => write!(f, "float"),
      Type::String => write!(f, "string"),
      Type::Regex => write!(f, "regex"),
      Type::Nothing => write!(f, "()"),

      Type::Fun(params, ret) => write!(
        f,
        "{} -> {}",
        params
          .iter()
          .map(maybe_add_parens)
          .collect::<Vec<String>>()
          .join(" "),
        ret
      ),

      Type::PartialTuple(index, element) => {
        write!(f, "({}: {}, ...)", index, element)
      }

      Type::Tuple(elements) => write!(
        f,
        "({})",
        elements
          .iter()
          .map(maybe_add_parens)
          .collect::<Vec<String>>()
          .join(", "),
      ),

      Type::Var(var) => {
        write!(f, "t{}", var)
        // attempt to convert the numeric var into an ascii letter, but
        // if it's >= 26, just go with t0, t1, ...
        // if *var >= 26 {
        //   return write!(f, "'t{}", var - 26);
        // }

        // write!(f, "'{}", char::from_u32((*var as u32) + 97).unwrap())
      }
    }
  }
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Type {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self)
  }
}
