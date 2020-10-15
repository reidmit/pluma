use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeDefNode {
  pub pos: Position,
  pub visibility: ExportVisibility,
  pub kind: TypeDefKind,
  pub name: TypeIdentifierNode,
  pub generic_type_constraints: GenericTypeConstraints,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct IntrinsicTypeDefNode {
  pub pos: Position,
  pub visibility: ExportVisibility,
  pub name: IdentifierNode,
  pub generic_type_constraints: GenericTypeConstraints,
}

#[cfg_attr(debug_assertions, derive(Debug))]
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
  // struct MyInt Int
  Struct {
    inner: TypeExprNode,
  },
  // trait Named .name :: String .getName() -> String
  Trait {
    fields: Vec<(IdentifierNode, TypeExprNode)>,
    methods: Vec<(Signature, TypeExprNode)>,
  },
}
