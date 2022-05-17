use super::*;

pub struct IfNode {
  pub pos: Position,
  pub subject: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for IfNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "if:{}-{} subject:({:#?}) pattern:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.subject, self.pattern, self.body
    )
  }
}
