use super::*;
use crate::common::*;
use crate::value_type::ValueType;

#[derive(Debug)]
pub struct ExprNode {
  pub pos: Position,
  pub kind: ExprKind,
  pub typ: ValueType,
}

#[derive(Debug)]
pub enum ExprKind {
  Assignment {
    left: Box<IdentifierNode>,
    right: Box<ExprNode>,
  },
  BinaryOperation {
    left: Box<ExprNode>,
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Block {
    params: Vec<IdentifierNode>,
    body: Vec<StatementNode>,
  },
  Call(CallNode),
  Dict(Vec<(ExprNode, ExprNode)>),
  EmptyTuple,
  FieldAccess {
    receiver: Box<ExprNode>,
    field: IdentifierNode,
  },
  Grouping(Box<ExprNode>),
  Identifier(IdentifierNode),
  MethodAccess {
    receiver: Box<ExprNode>,
    method_parts: Vec<IdentifierNode>,
  },
  MultiPartIdentifier(Vec<IdentifierNode>),
  Interpolation(Vec<ExprNode>),
  List(Vec<ExprNode>),
  Literal(LiteralNode),
  Match(MatchNode),
  RegExpr(RegExprNode),
  Tuple(Vec<ExprNode>),
  TypeAssertion {
    expr: Box<ExprNode>,
    asserted_type: TypeExprNode,
  },
  UnaryOperation {
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Underscore,
}
