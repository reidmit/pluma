use crate::binding::*;
use pluma_ast::*;
use std::collections::HashMap;

pub fn type_ident_to_value_type(node: &TypeIdentifierNode) -> ValueType {
  match &node.name[..] {
    "Int" => ValueType::Int,
    "Float" => ValueType::Float,
    "String" => ValueType::String,
    _ => ValueType::Named(node.name.clone()),
  }
}

pub fn type_expr_to_value_type(node: &TypeExprNode) -> ValueType {
  match &node.kind {
    TypeExprKind::EmptyTuple => ValueType::Nothing,
    TypeExprKind::Grouping(inner) => type_expr_to_value_type(&inner),
    TypeExprKind::Single(ident) => type_ident_to_value_type(&ident),
    TypeExprKind::UnlabeledTuple(entries) => {
      let mut entry_types = Vec::new();

      for entry in entries {
        entry_types.push(type_expr_to_value_type(entry));
      }

      ValueType::UnlabeledTuple(entry_types)
    }
    TypeExprKind::LabeledTuple(entries) => {
      let mut entry_types = Vec::new();

      for (label_ident, entry) in entries {
        entry_types.push((label_ident.name.clone(), type_expr_to_value_type(entry)));
      }

      ValueType::LabeledTuple(entry_types)
    }
    TypeExprKind::Func(param, ret) => {
      let param_type = type_expr_to_value_type(param);
      let return_type = type_expr_to_value_type(ret);

      ValueType::Func(vec![param_type], Box::new(return_type))
    }
  }
}

pub fn type_expr_to_struct_fields(node: &TypeExprNode) -> HashMap<String, Binding> {
  let mut fields = HashMap::new();

  match &node.kind {
    TypeExprKind::Grouping(inner) => {
      // recurse inside the parens
      return type_expr_to_struct_fields(inner);
    }
    TypeExprKind::Single(..) | TypeExprKind::EmptyTuple | TypeExprKind::Func(..) => {
      // only one field: .0
      fields.insert(
        "0".to_owned(),
        Binding {
          kind: BindingKind::Field,
          ref_count: 0,
          pos: node.pos,
          typ: type_expr_to_value_type(node),
        },
      );
    }
    TypeExprKind::UnlabeledTuple(entries) => {
      let mut i = 0;

      // one field per entry: .0, .1, .2, etc.
      for entry in entries {
        fields.insert(
          format!("{}", i),
          Binding {
            kind: BindingKind::Field,
            ref_count: 0,
            pos: entry.pos,
            typ: type_expr_to_value_type(entry),
          },
        );

        i += 1;
      }
    }
    TypeExprKind::LabeledTuple(entries) => {
      // one field per entry: .field1, .field2, .whatever, etc.
      for (label_ident, entry) in entries {
        fields.insert(
          format!("{}", label_ident.name),
          Binding {
            kind: BindingKind::Field,
            ref_count: 0,
            pos: entry.pos,
            typ: type_expr_to_value_type(entry),
          },
        );
      }
    }
  }

  fields
}
