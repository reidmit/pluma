use crate::scope::{Binding, BindingKind, Scope, TypeBindingKind};
use crate::type_utils;
use pluma_ast::*;
use pluma_diagnostics::*;
use pluma_visitor::*;
use std::collections::HashMap;

pub struct TypeCollector<'a> {
  pub scope: &'a mut Scope,
  pub diagnostics: Vec<Diagnostic>,
}

impl<'a> TypeCollector<'a> {
  pub fn new(scope: &'a mut Scope) -> Self {
    TypeCollector {
      scope,
      diagnostics: Vec::new(),
    }
  }

  fn diagnostic(&mut self, diag: Diagnostic) {
    self.diagnostics.push(diag)
  }

  fn check_result(&mut self, result: Result<(), Diagnostic>) {
    if let Err(diag) = result {
      self.diagnostic(diag);
    }
  }

  fn collect_def(
    &mut self,
    pos: Position,
    generic_type_constraints: &mut GenericTypeConstraints,
    kind: &mut DefKind,
    return_type: &Option<TypeExprNode>,
  ) {
    let mut constraints_map = HashMap::new();

    for (constraint_name, constraint_type_id) in generic_type_constraints {
      let constraint = if constraint_type_id.generics.is_empty() {
        TypeConstraint::NamedTrait(constraint_type_id.name.clone())
      } else {
        TypeConstraint::GenericTrait(
          constraint_type_id.name.clone(),
          constraint_type_id
            .generics
            .iter()
            .map(|type_expr| type_utils::type_expr_to_value_type(type_expr))
            .collect(),
        )
      };

      constraints_map.insert(
        constraint_name.name.clone(),
        ValueType::Constrained(constraint),
      );
    }

    match kind {
      DefKind::Function { signature } => {
        let mut name_parts = Vec::new();
        let mut param_types = Vec::new();

        for (part_name, part_type) in signature {
          name_parts.push(part_name.name.clone());

          let param_type = type_utils::type_expr_to_value_type(part_type);

          if let ValueType::Named(name) = &param_type {
            if let Some(constraint) = constraints_map.get(name) {
              param_types.push(constraint.clone());
            } else {
              param_types.push(param_type);
            }
          } else {
            param_types.push(param_type);
          }
        }

        let return_type = match return_type {
          Some(ret) => type_utils::type_expr_to_value_type(&ret),
          None => ValueType::Nothing,
        };

        let def_type = ValueType::Func(param_types, Box::new(return_type));
        let merged_name = name_parts.join(" ");

        self
          .scope
          .add_binding(BindingKind::Def, merged_name, def_type, pos);
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        let receiver_type = ValueType::Named(receiver.name.clone());

        let return_type = match return_type {
          Some(type_expr) => type_utils::type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let mut method_parts = Vec::new();
        let mut param_types = Vec::new();

        for (part_name, part_type_expr) in signature {
          method_parts.push(part_name.name.clone());
          param_types.push(type_utils::type_expr_to_value_type(part_type_expr));
        }

        let result = self.scope.add_type_method(
          receiver_type,
          method_parts,
          param_types,
          return_type,
          receiver.pos,
        );

        self.check_result(result);
      }

      DefKind::BinaryOperator { op, left, right } => {
        let receiver_type = type_utils::type_ident_to_value_type(left);

        let param_type = type_utils::type_ident_to_value_type(right);

        let return_type = match return_type {
          Some(type_expr) => type_utils::type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let method_parts = vec!["$".to_owned(), op.name.clone(), "$".to_owned()];
        let param_types = vec![param_type];

        let result = self.scope.add_type_method(
          receiver_type,
          method_parts,
          param_types,
          return_type,
          left.pos,
        );

        self.check_result(result);
      }

      DefKind::UnaryOperator { op, right } => {
        let receiver_type = type_utils::type_ident_to_value_type(right);

        let return_type = match return_type {
          Some(type_expr) => type_utils::type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let method_parts = vec![op.name.clone(), "$".to_owned()];
        let param_types = vec![];

        let result = self.scope.add_type_method(
          receiver_type,
          method_parts,
          param_types,
          return_type,
          right.pos,
        );

        self.check_result(result);
      }
    }
  }
}

impl<'a> VisitorMut for TypeCollector<'a> {
  fn enter_type_expr(&mut self, node: &mut TypeExprNode) {
    node.typ = type_utils::type_expr_to_value_type(node);
  }

  fn enter_def(&mut self, node: &mut DefNode) {
    self.collect_def(
      node.pos,
      &mut node.generic_type_constraints,
      &mut node.kind,
      &node.return_type,
    );
  }

  fn enter_intrinsic_def(&mut self, node: &mut IntrinsicDefNode) {
    self.collect_def(
      node.pos,
      &mut node.generic_type_constraints,
      &mut node.kind,
      &node.return_type,
    );
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
              let param_type = type_utils::type_expr_to_value_type(param_node);
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
          param_types.push(type_utils::type_expr_to_value_type(field_type));

          fields_map.insert(
            field_id.name.clone(),
            Binding {
              kind: BindingKind::Field,
              ref_count: 0,
              pos: field_id.pos,
              typ: type_utils::type_expr_to_value_type(field_type),
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

      TypeDefKind::Trait { fields, .. } => {
        let mut fields_map = HashMap::new();

        for field in fields {
          let (field_id, field_type) = field;

          fields_map.insert(
            field_id.name.clone(),
            Binding {
              kind: BindingKind::Field,
              ref_count: 0,
              pos: field_id.pos,
              typ: type_utils::type_expr_to_value_type(field_type),
            },
          );
        }

        self.scope.add_type_binding(
          typ.clone(),
          TypeBindingKind::Trait { fields: fields_map },
          node.name.pos,
        );
      }
    }
  }

  fn enter_intrinsic_type_def(&mut self, node: &mut IntrinsicTypeDefNode) {
    let intrinsic_type = match &node.name.name[..] {
      "Int" => Some(ValueType::Int),
      "Float" => Some(ValueType::Float),
      "String" => Some(ValueType::String),
      _ => None,
    };

    if let Some(typ) = intrinsic_type {
      self
        .scope
        .add_type_binding(typ, TypeBindingKind::IntrinsicType, node.name.pos);
    }
  }
}
