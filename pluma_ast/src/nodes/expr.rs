use super::*;
use crate::common::*;
use crate::value_type::ValueType;
use std::fmt;

pub struct ExprNode {
  pub pos: Position,
  pub kind: ExprKind,
  pub typ: ValueType,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum ExprKind {
  Access {
    receiver: Box<ExprNode>,
    property: Box<ExprNode>,
  },
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
  Grouping {
    inner: Box<ExprNode>,
  },
  Identifier {
    ident: IdentifierNode,
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
  QualifiedIdentifier {
    qualifier: QualifierNode,
    ident: Box<IdentifierNode>,
  },
  QualifiedMultiPartIdentifier {
    qualifier: QualifierNode,
    parts: Vec<IdentifierNode>,
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

#[cfg(debug_assertions)]
impl fmt::Debug for ExprNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Expr{:?} ", self.pos)?;

    match &self.kind {
      ExprKind::Call { call } => write!(f, "{:#?}", call),
      ExprKind::Block { block } => write!(f, "{:#?}", block),
      ExprKind::Literal { literal } => write!(f, "{:?}", literal),
      ExprKind::Identifier { ident } => write!(f, "{:?}", ident),
      ExprKind::QualifiedIdentifier { qualifier, ident } => {
        write!(f, "{:?} {:?}", qualifier, ident)
      }
      ExprKind::MultiPartIdentifier { parts } => write!(f, "MultiPartIdent {:?}", parts),
      ExprKind::QualifiedMultiPartIdentifier { qualifier, parts } => {
        write!(f, "MultiPartIdent {:?} {:?}", qualifier, parts)
      }
      ExprKind::UnlabeledTuple { entries } => write!(f, "UnlabeledTuple {:#?}", entries),
      ExprKind::LabeledTuple { entries } => write!(f, "LabeledTuple {:#?}", entries),
      _ => write!(f, "{:#?}", self.kind),
    }
  }
}
