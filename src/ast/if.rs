use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct IfNode {
  pub span: Span,
  pub subject: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}
