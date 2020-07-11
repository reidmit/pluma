use pluma_ast::*;
use std::collections::HashMap;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Binding {
  pub typ: ValueType,
  pub ref_count: usize,
  pub pos: (usize, usize),
  pub kind: BindingKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeBinding {
  pub ref_count: usize,
  pub pos: (usize, usize),
  pub kind: TypeBindingKind,
  pub methods: HashMap<Vec<String>, ValueType>,
}

#[derive(PartialEq)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum BindingKind {
  Const,
  Let,
  Def,
  Param,
  EnumVariant,
  StructConstructor,
  Field,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeBindingKind {
  Enum,
  Struct { fields: HashMap<String, Binding> },
  Alias,
  Trait { fields: HashMap<String, Binding> },
  IntrinsicType,
}
