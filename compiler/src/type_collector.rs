use crate::diagnostics::Diagnostic;
use crate::scope::{Binding, BindingKind, Scope, TypeBindingKind};
use crate::visitor::Visitor;
use pluma_ast::nodes::*;
use pluma_ast::value_type::ValueType;
use std::collections::HashMap;

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
  fn enter_type_expr(&mut self, node: &mut TypeExprNode) {
    node.typ = self.type_expr_to_value_type(node);
  }

  fn enter_def(&mut self, node: &mut DefNode) {
    match &node.kind {
      DefKind::Function { signature } => {
        let mut name_parts = Vec::new();
        let mut param_types = Vec::new();

        for (part_name, part_type) in signature {
          name_parts.push(part_name.name.clone());
          param_types.push(self.type_expr_to_value_type(part_type));
        }

        let return_type = match &node.return_type {
          Some(ret) => self.type_expr_to_value_type(&ret),
          None => ValueType::Nothing,
        };

        let def_type = ValueType::Func(param_types, Box::new(return_type));
        let merged_name = name_parts.join(" ");

        self
          .scope
          .add_binding(BindingKind::Def, merged_name, def_type, node.pos);
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        let receiver_type = ValueType::Named(receiver.name.clone());

        let return_type = match &node.return_type {
          Some(type_expr) => self.type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let mut method_parts = Vec::new();
        let mut param_types = Vec::new();

        for (part_name, part_type_expr) in signature {
          method_parts.push(part_name.name.clone());
          param_types.push(self.type_expr_to_value_type(part_type_expr));
        }

        self
          .scope
          .add_type_method(receiver_type, method_parts, param_types, return_type)
      }

      _ => {}
    }
  }

  fn enter_intrinsic_type_def(&mut self, node: &mut IntrinsicTypeDefNode) {
    let intrinsic_type = match &node.name.name[..] {
      "Int" => Some(ValueType::Named("Int".to_owned())),
      "String" => Some(ValueType::Named("String".to_owned())),
      _ => None,
    };

    if let Some(typ) = intrinsic_type {
      self
        .scope
        .add_type_binding(typ, TypeBindingKind::IntrinsicType, node.name.pos);
    }
  }

  fn enter_type_def(&mut self, node: &mut TypeDefNode) {
    let typ = ValueType::Named(node.name.name.clone());

    match &node.kind {
      TypeDefKind::Enum { variants } => {
        self
          .scope
          .add_type_binding(typ.clone(), TypeBindingKind::Enum, node.name.pos);

        for variant in variants {
          match &variant.kind {
            EnumVariantKind::Identifier(ident_node) => {
              let variant_name = ident_node.name.clone();
              let variant_type = typ.clone();

              self.scope.add_binding(
                BindingKind::EnumVariant,
                variant_name,
                variant_type,
                ident_node.pos,
              );
            }

            EnumVariantKind::Constructor(constructor_node, param_node) => {
              let constructor_name = constructor_node.name.clone();
              let param_type = self.type_expr_to_value_type(param_node);
              let constructor_type = ValueType::Func(vec![param_type], Box::new(typ.clone()));

              self.scope.add_binding(
                BindingKind::EnumVariant,
                constructor_name,
                constructor_type,
                variant.pos,
              );
            }
          }
        }
      }

      TypeDefKind::Struct { fields } => {
        let mut param_types = Vec::new();
        let mut fields_map = HashMap::new();

        for field in fields {
          let (field_id, field_type) = field;
          param_types.push(self.type_expr_to_value_type(field_type));

          fields_map.insert(
            field_id.name.clone(),
            Binding {
              kind: BindingKind::Field,
              ref_count: 0,
              pos: field_id.pos,
              typ: self.type_expr_to_value_type(field_type),
            },
          );
        }

        self.scope.add_type_binding(
          typ.clone(),
          TypeBindingKind::Struct { fields: fields_map },
          node.name.pos,
        );

        let param_tuple_type = ValueType::Tuple(param_types);
        let constructor_type = ValueType::Func(vec![param_tuple_type], Box::new(typ));

        self.scope.add_binding(
          BindingKind::StructConstructor,
          node.name.name.clone(),
          constructor_type,
          node.name.pos,
        );
      }

      TypeDefKind::Alias { .. } => {
        self
          .scope
          .add_type_binding(typ.clone(), TypeBindingKind::Alias, node.name.pos);
      }

      TypeDefKind::Trait { .. } => {
        self
          .scope
          .add_type_binding(typ.clone(), TypeBindingKind::Trait, node.name.pos);
      }
    }
  }
}
