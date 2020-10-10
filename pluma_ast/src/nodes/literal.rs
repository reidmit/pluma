use crate::common::*;
use std::fmt;

pub struct LiteralNode {
  pub pos: Position,
  pub kind: LiteralKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum LiteralKind {
  FloatDecimal(f64),
  IntDecimal(i32),
  IntOctal(i32),
  IntHex(i32),
  IntBinary(i32),
  Str(String),
}

#[cfg(debug_assertions)]
impl fmt::Debug for LiteralNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Literal{:?} {:?}", self.pos, self.kind)
  }
}
