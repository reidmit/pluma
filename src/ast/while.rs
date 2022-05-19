use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct WhileNode {
  pub loc: Location,
  pub condition: Box<ExprNode>,
  pub pattern: PatternNode,
  pub body: Vec<ExprNode>,
}
