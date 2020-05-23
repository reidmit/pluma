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

  fn type_expr_to_value_type(&self, node: &TypeExprNode) -> ValueType {
    match &node.kind {
      TypeExprKind::EmptyTuple => ValueType::Nothing,
      TypeExprKind::Grouping(inner) => self.type_expr_to_value_type(&inner),
      TypeExprKind::Single(ident) => ValueType::Named(ident.name.clone()),
      TypeExprKind::Tuple(entries) => {
        let mut entry_types = Vec::new();

        for entry in entries {
          entry_types.push(self.type_expr_to_value_type(entry));
        }

        ValueType::Tuple(entry_types)
      }
      TypeExprKind::Func(param, ret) => {
        let param_type = self.type_expr_to_value_type(param);
        let return_type = self.type_expr_to_value_type(ret);

        ValueType::Func(vec![param_type], Box::new(return_type))
      }
    }
  }
}

impl<'a> Visitor for TypeCollector<'a> {
  fn enter_type_def(&mut self, node: &mut TypeDefNode) {
    let typ = ValueType::Named(node.name.name.clone());

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

      TypeDefKind::Struct { fields } => {
        let mut param_types = Vec::new();

        for field in fields {
          let (field_name, field_type) = field;
          let value_type = self.type_expr_to_value_type(field_type);
          param_types.push(value_type);
        }

        let param_tuple_type = ValueType::Tuple(param_types);

        // Structs introduce a new constructor function
        let constructor_type = ValueType::Func(vec![param_tuple_type], Box::new(typ));

        self
          .scope
          .add_let_binding(node.name.name.clone(), constructor_type, node.name.pos)
      }

      TypeDefKind::Alias { of } => {}

      TypeDefKind::Trait { fields, methods } => {}
    }
  }
}
