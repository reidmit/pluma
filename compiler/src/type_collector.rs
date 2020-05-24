use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::scope::{BindingKind, Scope, TypeBindingKind};
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
      self.scope.add_type_binding(
        TypeBindingKind::IntrinsicType,
        node.name.name.clone(),
        typ,
        node.name.pos,
      );
    }
  }

  fn enter_type_def(&mut self, node: &mut TypeDefNode) {
    let typ = ValueType::Named(node.name.name.clone());

    match &node.kind {
      TypeDefKind::Enum { variants } => {
        self.scope.add_type_binding(
          TypeBindingKind::Enum,
          node.name.name.clone(),
          typ.clone(),
          node.name.pos,
        );

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
        self.scope.add_type_binding(
          TypeBindingKind::Struct,
          node.name.name.clone(),
          typ.clone(),
          node.name.pos,
        );

        let mut param_types = Vec::new();

        for field in fields {
          let (field_name, field_type) = field;
          let value_type = self.type_expr_to_value_type(field_type);
          param_types.push(value_type);
        }

        let param_tuple_type = ValueType::Tuple(param_types);
        let constructor_type = ValueType::Func(vec![param_tuple_type], Box::new(typ));

        self.scope.add_binding(
          BindingKind::StructConstructor,
          node.name.name.clone(),
          constructor_type,
          node.name.pos,
        )
      }

      TypeDefKind::Alias { of } => {
        self.scope.add_type_binding(
          TypeBindingKind::Alias,
          node.name.name.clone(),
          typ.clone(),
          node.name.pos,
        );
      }

      TypeDefKind::Trait { fields, methods } => {
        self.scope.add_type_binding(
          TypeBindingKind::Trait,
          node.name.name.clone(),
          typ.clone(),
          node.name.pos,
        );
      }
    }
  }
}
