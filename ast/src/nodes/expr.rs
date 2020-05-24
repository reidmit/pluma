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
  Chain {
    receiver: Box<ExprNode>,
    prop: Box<ExprNode>,
  },
  Dict(Vec<(ExprNode, ExprNode)>),
  EmptyTuple,
  Grouping(Box<ExprNode>),
  Identifier(IdentifierNode),
  MultiPartIdentifier(Vec<IdentifierNode>),
  Interpolation(Vec<ExprNode>),
  List(Vec<ExprNode>),
  Literal(LiteralNode),
  Match(MatchNode),
  Tuple(Vec<ExprNode>),
  UnaryOperation {
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Underscore,
}
