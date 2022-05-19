use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct WhenNode {
  pub loc: Location,
  pub subject: Box<ExprNode>,
  pub cases: Vec<CaseNode>,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CaseNode {
  pub loc: Location,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}
