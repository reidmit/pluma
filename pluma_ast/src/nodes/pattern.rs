use super::*;
use crate::common::*;
use std::fmt;

pub struct PatternNode {
  pub pos: Position,
  pub kind: PatternKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum PatternKind {
  // e.g. let x = / let mut x
  Identifier(IdentifierNode, bool),
  // e.g. let Person (x, y) =
  Constructor(IdentifierNode, Box<PatternNode>),
  // e.g. let (x, y) =
  UnlabeledTuple(Vec<PatternNode>),
  // e.g. let (x: a, y: b) =
  LabeledTuple(Vec<(IdentifierNode, PatternNode)>),
  // e.g. '_' in let (x, _) =
  Underscore,
  // e.g. '1' in match x | 1 => "yes" | _ => "no"
  Literal(LiteralNode),
  // e.g. match str | "$(thing)?" => "yes" | _ => "no"
  Interpolation(Vec<ExprNode>),
}

#[cfg(debug_assertions)]
impl fmt::Debug for PatternNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Pattern{:?} {:?}", self.pos, self.kind)
  }
}
