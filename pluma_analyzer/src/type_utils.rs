use pluma_ast::*;

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
    TypeExprKind::Tuple(entries) => {
      let mut entry_types = Vec::new();

      for entry in entries {
        entry_types.push(type_expr_to_value_type(entry));
      }

      ValueType::UnlabeledTuple(entry_types)
    }
    TypeExprKind::Func(param, ret) => {
      let param_type = type_expr_to_value_type(param);
      let return_type = type_expr_to_value_type(ret);

      ValueType::Func(vec![param_type], Box::new(return_type))
    }
  }
}
