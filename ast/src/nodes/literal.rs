use crate::common::*;
use crate::value_type::ValueType;

#[derive(Debug)]
pub struct LiteralNode {
  pub pos: Position,
  pub kind: LiteralKind,
  pub typ: ValueType,
}

#[derive(Debug)]
pub enum LiteralKind {
  FloatDecimal(f64),
  IntDecimal(i128),
  IntOctal(i128),
  IntHex(i128),
  IntBinary(i128),
  Str(String),
}
