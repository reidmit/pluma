use super::*;

pub struct IfNode {
  pub pos: Position,
  pub discriminant: Box<ExprNode>,
  pub cases: Vec<IfCaseNode>,
}

pub struct IfCaseNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub body: ExprNode,
}

impl std::fmt::Debug for IfNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "if:{}-{} discriminant:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.discriminant, self.cases
    )
  }
}

impl std::fmt::Debug for IfCaseNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "case:{}-{} ({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.pattern, self.body
    )
  }
}
