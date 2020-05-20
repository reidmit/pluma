use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::scope::Scope;
use crate::types::ValueType;
use crate::visitor::Visitor;

pub struct TypeCollector<'a> {
  pub diagnostics: Vec<Diagnostic>,
  pub scope: &'a mut Scope,
}

impl<'a> TypeCollector<'a> {
  pub fn new(scope: &'a mut Scope) -> Self {
    TypeCollector {
      diagnostics: Vec::new(),
      scope,
    }
  }
}

impl<'a> Visitor for TypeCollector<'a> {
  fn enter_type_def(&mut self, node: &mut TypeDefNode) {
    // let typ = ValueType::Named(node.name.name.clone());

    match &node.kind {
      TypeDefKind::Enum { variants } => {
        for variant in variants {
          // match &variant.kind {
          //   TypeExprKind::Constructor(ident_node) => {
          //     println!("variant: {:#?}", ident_node.name);

          //     let variant_name = ident_node.name.clone();
          //     let variant_type = typ.clone();

          //     self
          //       .scope
          //       .add_let_binding(variant_name, variant_type, ident_node.pos);
          //   }
          //   _ => todo!("other variants"),
          // }
        }
      }
      _ => todo!("other type defs"),
    }
  }
}
