use super::*;

pub struct CaseNode {
  pub pos: Position,
  pub discriminant: Box<ExprNode>,
  pub cases: Vec<CaseBranchNode>,
}

pub struct CaseBranchNode {
  pub pos: Position,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}

impl std::fmt::Debug for CaseNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "when:{}-{} discriminant:({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.discriminant, self.cases
    )
  }
}

impl std::fmt::Debug for CaseBranchNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "case:{}-{} ({:#?}) {:#?}",
      self.pos.0, self.pos.1, self.pattern, self.body
    )
  }
}
