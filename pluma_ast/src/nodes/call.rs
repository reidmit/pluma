use super::*;
use crate::common::*;
use crate::value_type::ValueType;
use std::fmt;

pub struct CallNode {
  pub pos: Position,
  pub callee: Box<ExprNode>,
  pub args: Vec<ExprNode>,
  pub typ: ValueType,
}

#[cfg(debug_assertions)]
impl fmt::Debug for CallNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("Call")
      .field("callee", &self.callee)
      .field("args", &self.args)
      .finish()
  }
}
