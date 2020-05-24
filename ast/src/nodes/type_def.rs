use super::*;
use crate::common::*;

#[derive(Debug)]
pub struct TypeDefNode {
  pub pos: Position,
  pub kind: TypeDefKind,
  pub name: TypeIdentifierNode,
  pub generic_type_constraints: GenericTypeConstraints,
}

#[derive(Debug)]
pub struct IntrinsicTypeDefNode {
  pub pos: Position,
  pub name: IdentifierNode,
  pub generic_type_constraints: GenericTypeConstraints,
}

#[derive(Debug)]
pub enum TypeDefKind {
  // alias StringList List<String>
  Alias {
    of: TypeExprNode,
  },
  // enum Color | Red | Green | Blue
  Enum {
    variants: Vec<EnumVariantNode>,
  },
  // struct Person (name :: String, age :: Int)
  Struct {
    fields: Vec<(IdentifierNode, TypeExprNode)>,
  },
  // trait Named .name :: String .getName() -> String
  Trait {
    fields: Vec<(IdentifierNode, TypeExprNode)>,
    methods: Vec<(Signature, TypeExprNode)>,
  },
}
