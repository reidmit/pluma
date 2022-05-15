use super::*;

pub struct WhenNode {
  pub pos: Position,
  pub subject: Box<ExprNode>,
  pub cases: Vec<CaseNode>,
}

pub struct CaseNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for WhenNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "when:{}-{} subject:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.subject, self.cases
    )
  }
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for CaseNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "case:{}-{} ({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.pattern, self.body
    )
  }
}
