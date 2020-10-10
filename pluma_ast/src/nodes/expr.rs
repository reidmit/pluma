use super::*;
use crate::common::*;
use crate::value_type::ValueType;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ExprNode {
  pub pos: Position,
  pub kind: ExprKind,
  pub typ: ValueType,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ExprKind {
  Assignment {
    left: Box<ExprNode>,
    right: Box<ExprNode>,
  },
  BinaryOperation {
    left: Box<ExprNode>,
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Block {
    block: BlockNode,
  },
  Call {
    call: CallNode,
  },
  Dict {
    entries: Vec<(ExprNode, ExprNode)>,
  },
  EmptyTuple,
  FieldAccess {
    receiver: Box<ExprNode>,
    field: Box<ExprNode>,
  },
  Grouping {
    inner: Box<ExprNode>,
  },
  Identifier {
    ident: IdentifierNode,
  },
  MethodAccess {
    receiver: Box<ExprNode>,
    method_parts: Vec<IdentifierNode>,
  },
  MultiPartIdentifier {
    parts: Vec<IdentifierNode>,
  },
  Interpolation {
    parts: Vec<ExprNode>,
  },
  List {
    elements: Vec<ExprNode>,
  },
  Literal {
    literal: LiteralNode,
  },
  Match {
    match_: MatchNode,
  },
  RegExpr {
    regex: RegExprNode,
  },
  LabeledTuple {
    entries: Vec<(IdentifierNode, ExprNode)>,
  },
  UnlabeledTuple {
    entries: Vec<ExprNode>,
  },
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
