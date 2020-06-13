use pluma_ast::nodes::*;
use pluma_ast::value_type::*;

pub fn type_ident_to_value_type(node: &TypeIdentifierNode) -> ValueType {
  ValueType::Named(node.name.clone())
}

pub fn type_expr_to_value_type(node: &TypeExprNode) -> ValueType {
  match &node.kind {
    TypeExprKind::EmptyTuple => ValueType::Nothing,
    TypeExprKind::Grouping(inner) => type_expr_to_value_type(&inner),
    TypeExprKind::Single(ident) => ValueType::Named(ident.name.clone()),
    TypeExprKind::Tuple(entries) => {
      let mut entry_types = Vec::new();

      for entry in entries {
        entry_types.push(type_expr_to_value_type(entry));
      }

      ValueType::Tuple(entry_types)
    }
    TypeExprKind::Func(param, ret) => {
      let param_type = type_expr_to_value_type(param);
      let return_type = type_expr_to_value_type(ret);

      ValueType::Func(vec![param_type], Box::new(return_type))
    }
  }
}