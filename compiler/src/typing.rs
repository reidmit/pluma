use std::collections::HashMap;

#[derive(Clone)]
pub enum Type {
  Unknown,
  Var(usize),
  Int,
  Float,
  Regex,
  String,
  Nothing,
  Fun(Vec<Type>, Box<Type>),
}

impl Type {
  pub fn contains_var(&self, var: usize) -> bool {
    match &self {
      Type::Var(n) => var == *n,

      Type::Nothing | Type::Int | Type::Float | Type::String | Type::Regex | Type::Unknown => false,

      Type::Fun(param_types, return_type) => {
        for param_type in param_types {
          if param_type.contains_var(var) {
            return true;
          }
        }

        return return_type.contains_var(var);
      }

      _ => false, // TODO: ??
    }
  }
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeScheme {
  Var(usize),
  Forall(Vec<usize>, Type),
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeConstraint {
  Eq(Type, Type),
  Gen(TypeScheme, Type),
  Inst(usize, Type),
}

impl std::fmt::Display for Type {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Type::Unknown => write!(f, "?"),
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
          .map(|p| {
            let s = format!("{}", p);
            if s.contains(" ") {
              format!("({})", s)
            } else {
              s
            }
          })
          .collect::<Vec<String>>()
          .join(" "),
        ret
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

pub struct TypeSubstitution {
  pub solutions: HashMap<usize, Type>,
}

impl TypeSubstitution {
  pub fn empty() -> Self {
    Self {
      solutions: HashMap::new(),
    }
  }

  pub fn with_entry(key: usize, value: Type) -> Self {
    let mut solutions = HashMap::with_capacity(1);
    solutions.insert(key, value);
    Self { solutions }
  }

  pub fn apply_to_type(&self, ty: &Type) -> Type {
    match ty {
      Type::Var(var) if self.solutions.contains_key(var) => {
        self.solutions.get(var).unwrap().clone()
      }

      Type::Fun(param_types, return_type) => Type::Fun(
        param_types.iter().map(|p| self.apply_to_type(p)).collect(),
        self.apply_to_type(return_type).into(),
      ),

      other => (*other).clone(),
    }
  }

  pub fn apply_to_constraints(&self, constraints: &[TypeConstraint]) -> Vec<TypeConstraint> {
    use TypeConstraint::*;

    constraints
      .iter()
      .map(|con| match con {
        Eq(a, b) => Eq(self.apply_to_type(a), self.apply_to_type(b)),
        // TODO: should we have a context arg here as well?
        // see https://github.com/igstan/linguae/blob/7e806dd121c21ed35187377fe3bd92d29d6150e6/lingua-002-hm-inference-sml/src/constraint.sml#L21
        Gen(scheme, ty) => Gen(scheme.clone(), self.apply_to_type(ty)),
        Inst(var, ty) => Inst(*var, self.apply_to_type(ty)),
      })
      .collect()
  }

  pub fn compose(&self, other: TypeSubstitution) -> TypeSubstitution {
    let mut merged_solutions = HashMap::new();

    for (k, v) in &self.solutions {
      // add self.solutions with replacements from other
      merged_solutions.insert(*k, other.apply_to_type(v));
    }

    for (k, v) in &other.solutions {
      // add other.solutions
      merged_solutions.insert(*k, v.clone());
    }

    TypeSubstitution {
      solutions: merged_solutions,
    }
  }
}
