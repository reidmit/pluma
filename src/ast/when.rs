use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct WhenNode {
  pub span: Span,
  pub subject: Box<ExprNode>,
  pub cases: Vec<CaseNode>,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CaseNode {
  pub span: Span,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}
