use super::*;
use crate::value_type::*;

pub struct PatternNode {
  pub pos: Position,
  pub kind: PatternKind,
}

pub enum PatternKind {
  // e.g. if val is x then ...
  Identifier(IdentifierNode),
  // e.g. if val is person _ then ...
  Constructor(IdentifierNode, Box<PatternNode>),
  // e.g. if val is (a: 1, b: 2) then ...
  Tuple(Vec<(Option<IdentifierNode>, PatternNode)>),
  // e.g. if val is _ then ...
  Underscore,
  // e.g. if val is 1 then ...
  Literal(LiteralNode),
  // e.g. if val is "$(thing)?" then ...
  Interpolation(Vec<ExprNode>),
}

impl PatternNode {
  pub fn to_expr(self) -> ExprNode {
    let pos = self.pos;

    let expr_kind = match self.kind {
      PatternKind::Identifier(ident) => ExprKind::Identifier(ident),

      PatternKind::Literal(literal) => ExprKind::Literal(literal),

      PatternKind::Interpolation(parts) => ExprKind::Interpolation(parts),

      PatternKind::Tuple(entry_patterns) => {
        let mut entries = Vec::new();

        for (label, pat) in entry_patterns {
          entries.push(TupleEntry(label, pat.to_expr()))
        }

        ExprKind::Tuple(entries)
      }

      PatternKind::Constructor(ident, arg) => {
        let callee = ExprNode {
          pos: ident.pos,
          kind: ExprKind::Identifier(ident),
          resolved_type: ValueType::Unknown,
        };

        let arg_expr = arg.to_expr();

        let call = CallNode {
          pos,
          callee: Box::new(callee),
          args: vec![arg_expr],
        };

        ExprKind::Call(call)
      }

      other => todo!("other expr kind in pattern: {:#?}", other),
    };

    ExprNode {
      pos,
      kind: expr_kind,
      resolved_type: ValueType::Unknown,
    }
  }
}

impl std::fmt::Debug for PatternNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "pattern:{}-{} {:#?}", self.pos.0, self.pos.1, self.kind)
  }
}

impl std::fmt::Debug for PatternKind {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use PatternKind::*;

    match &self {
      Identifier(ident) => write!(f, "{:?}", ident),
      Constructor(ctor, arg_pattern) => write!(f, "constructor {:?} ({:#?})", ctor, arg_pattern),
      Tuple(elem_patterns) => write!(f, "tuple ({:#?})", elem_patterns),
      Underscore => write!(f, "wildcard"),
      Literal(lit) => write!(f, "literal ({:#?})", lit),
      Interpolation(parts) => write!(f, "interpolation ({:#?})", parts),
    }
  }
}
