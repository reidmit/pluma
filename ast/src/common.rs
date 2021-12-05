use crate::nodes::*;

pub type Position = (usize, usize);

pub type SignaturePart = (IdentifierNode, Box<TypeExprNode>);

pub type Signature = Vec<SignaturePart>;

pub type GenericTypeConstraints = Vec<(IdentifierNode, TypeIdentifierNode)>;
