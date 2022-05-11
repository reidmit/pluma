use super::*;

pub struct WhenNode {
  pub pos: Position,
  pub condition: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}

impl std::fmt::Debug for WhenNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "when:{}-{} condition:({:#?}) pattern:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.condition, self.pattern, self.body
    )
  }
}
