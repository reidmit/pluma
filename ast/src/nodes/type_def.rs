use super::*;
use crate::common::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeDefNode {
  pub pos: Position,
  pub name: TypeIdentifierNode,
  pub kind: TypeDefKind,
  pub generic_type_constraints: GenericTypeConstraints,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeDefKind {
  // type string-list = list<string>
  Alias {
    of: TypeExprNode,
  },
  // type color = enum { red, green, blue }
  Enum {
    variants: Vec<EnumVariantNode>,
  },
  // type person = struct (name :: string, age :: int)
  // type my-int = struct int
  Struct {
    inner: TypeExprNode,
  },
  // type named = trait { .name :: string, | get-name _ :: self -> string }
  Trait {
    fields: Vec<(IdentifierNode, TypeExprNode)>,
    methods: Vec<(Signature, TypeExprNode)>,
  },
}
