use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeExprNode {
  pub span: Span,
  pub kind: TypeExprKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeExprKind {
  // e.g. string or dict<int, string>
  Single(TypeIdentifierNode),
  // e.g. fn string int -> bool
  Func(Vec<TypeExprNode>, Box<TypeExprNode>),
  // e.g. (string, bool)
  Tuple(Vec<TypeExprNode>),
  // e.g. {a: string, b: bool}
  Record(Vec<(IdentifierNode, TypeExprNode)>),
  // e.g. ()
  EmptyTuple,
  // e.g. (string) or (fn string -> bool)
  Grouping(Box<TypeExprNode>),
}
