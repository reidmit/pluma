use crate::types::*;

#[derive(Clone)]
pub enum Scheme {
  Var(usize),
  Forall(Vec<usize>, Type),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Scheme {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Scheme::Var(var) => write!(f, "scheme {}", Type::Var(*var)),
      Scheme::Forall(vars, ty) => write!(
        f,
        "forall{} . {}",
        vars
          .iter()
          .map(|v| format!(" {}", v))
          .collect::<Vec<String>>()
          .join(""),
        ty
      ),
    }
  }
}
