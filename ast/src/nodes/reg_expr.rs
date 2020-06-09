use crate::common::*;

#[derive(Debug)]
pub struct RegExprNode {
  pub pos: Position,
  pub kind: RegExprKind,
}

#[derive(Debug)]
pub enum RegExprKind {
  Literal(String),
  CharacterClass(String),
  OneOrMore(Box<RegExprNode>),
  ZeroOrMore(Box<RegExprNode>),
  OneOrZero(Box<RegExprNode>),
  AtLeastCount(Box<RegExprNode>, usize),
  AtMostCount(Box<RegExprNode>, usize),
  ExactCount(Box<RegExprNode>, usize),
  RangeCount(Box<RegExprNode>, usize, usize),
  Grouping(Box<RegExprNode>),
  Sequence(Vec<RegExprNode>),
  Alternation(Vec<RegExprNode>),
  NamedCapture(String, Box<RegExprNode>),
}
