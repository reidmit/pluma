use super::*;

pub struct WhileNode {
  pub pos: Position,
  pub condition: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for WhileNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "while:{}-{} condition:({:#?}) pattern:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.condition, self.pattern, self.body
    )
  }
}
