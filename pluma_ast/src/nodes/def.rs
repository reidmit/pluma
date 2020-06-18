use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DefNode {
  pub pos: Position,
  pub kind: DefKind,
  pub return_type: Option<TypeExprNode>,
  pub generic_type_constraints: GenericTypeConstraints,
  pub params: Vec<IdentifierNode>,
  pub body: Vec<StatementNode>,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct IntrinsicDefNode {
  pub pos: Position,
  pub kind: DefKind,
  pub return_type: Option<TypeExprNode>,
  pub generic_type_constraints: GenericTypeConstraints,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum DefKind {
  // def hi(A, B) -> Ret { ... }
  Function {
    signature: Signature,
  },
  // def (Receiver).hi() -> Ret { ... }
  Method {
    receiver: Box<TypeIdentifierNode>,
    signature: Signature,
  },
  // def (A) ++ (B) -> Ret { ... }
  BinaryOperator {
    left: Box<TypeIdentifierNode>,
    op: Box<OperatorNode>,
    right: Box<TypeIdentifierNode>,
  },
  // def ~(A) -> Ret { ... }
  UnaryOperator {
    op: Box<OperatorNode>,
    right: Box<TypeIdentifierNode>,
  },
}
