use crate::types::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct ValueBinding {
  pub ty_scheme: Scheme,
  pub ref_count: usize,
  pub span: (usize, usize),
}

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct TypeBinding {
  pub ty: Type,
  pub span: (usize, usize),
}
