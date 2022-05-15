use super::*;

pub struct IfNode {
  pub pos: Position,
  pub condition: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}

impl std::fmt::Debug for IfNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "if:{}-{} condition:({:#?}) pattern:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.condition, self.pattern, self.body
    )
  }
}
