use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
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
