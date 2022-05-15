use super::*;

pub struct ForNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub data: Box<ExprNode>,
  pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ForNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "for:{}-{} pattern:({:#?}) data:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.pattern, self.data, self.body
    )
  }
}
