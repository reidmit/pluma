use super::*;
use crate::expr_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeExprNode {
  pub pos: Position,
  pub kind: TypeExprKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeExprKind {
  // e.g. string or dict<int, string>
  Single(TypeIdentifierNode),
  // e.g. fn string int -> bool
  Func(Vec<TypeExprNode>, Box<TypeExprNode>),
  // e.g. (string, bool)
  Tuple(Vec<TypeExprNode>),
  // e.g. {a: string, b: bool}
  Record(Vec<(IdentifierNode, TypeExprNode)>),
  // e.g. ()
  EmptyTuple,
  // e.g. (string) or (fn string -> bool)
  Grouping(Box<TypeExprNode>),
}

impl TypeExprNode {
  pub fn to_type(&self) -> ExprType {
    use TypeExprKind::*;

    match &self.kind {
      EmptyTuple => ExprType::Nothing,

      Grouping(type_expr) => type_expr.to_type(),

      Single(type_ident) => {
        if type_ident.generics.is_empty() {
          // check if it's a built-in type:
          match &type_ident.name[..] {
            "string" => ExprType::String,
            "int" => ExprType::Int,
            "regex" => ExprType::Regex,
            "float" => ExprType::Float,
            "nothing" => ExprType::Nothing,
            "bool" => ExprType::Bool,
            _ => ExprType::Named(type_ident.name.clone()),
          }
        } else {
          let params = type_ident.generics.iter().map(|g| g.to_type()).collect();
          ExprType::NamedWithParams(type_ident.name.clone(), params)
        }
      }

      Func(params, returned) => ExprType::Func(
        params.iter().map(|p| p.to_type()).collect(),
        Box::new(returned.to_type()),
      ),

      Tuple(entries) => ExprType::Tuple(entries.iter().map(|t_expr| t_expr.to_type()).collect()),

      Record(entries) => ExprType::Record(
        entries
          .iter()
          .map(|(label, t_expr)| (label.name.clone(), t_expr.to_type()))
          .collect(),
      ),
    }
  }
}
