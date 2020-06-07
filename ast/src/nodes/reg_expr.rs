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
  Grouping(Box<RegExprNode>),
  Sequence(Vec<RegExprNode>),
  Alternation(Vec<RegExprNode>),
  NamedCapture(String, Box<RegExprNode>),
}
