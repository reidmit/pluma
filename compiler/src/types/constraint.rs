use crate::types::*;

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Constraint {
  Eq(Type, Type),
  Gen(Scheme, Type),
  Inst(usize, Type),
}
