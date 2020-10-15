use crate::nodes::*;

pub type Position = (usize, usize);

pub type SignaturePart = (IdentifierNode, Box<TypeExprNode>);

pub type Signature = Vec<SignaturePart>;

pub type GenericTypeConstraints = Vec<(IdentifierNode, TypeIdentifierNode)>;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone, Copy, PartialEq)]
pub enum ExportVisibility {
  Public = 0,
  Internal,
  Private,
}
