use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct WhileNode {
  pub span: Span,
  pub condition: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}
