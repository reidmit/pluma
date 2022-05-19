use super::*;
use crate::expr_type::*;

pub struct PatternNode {
  pub pos: Position,
  pub kind: PatternKind,
}

pub enum PatternKind {
  // e.g. if val is x { ... }
  Identifier(IdentifierNode),
  // e.g. if val is enum-variant _ { ... }
  Constructor(IdentifierNode, Box<PatternNode>),
  // e.g. if val is (a, b) { ... }
  Tuple(Vec<PatternNode>),
  // e.g. if val is {a: 1, b: 2} { ... }
  Record(Vec<(IdentifierNode, PatternNode)>),
  // e.g. if val is _ { ... }
  Underscore,
  // e.g. if val is 1 { ... }
  Literal(LiteralNode),
  // e.g. if name is "${first} ${last}" { ... }
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

        for pat in entry_patterns {
          entries.push(pat.to_expr())
        }

        ExprKind::Tuple(entries)
      }

      PatternKind::Record(entry_patterns) => {
        let mut entries = Vec::new();

        for (label, pat) in entry_patterns {
          entries.push((label, pat.to_expr()))
        }

        ExprKind::Record(entries)
      }

      PatternKind::Constructor(ident, arg) => {
        let callee = ExprNode {
          pos: ident.pos,
          kind: ExprKind::Identifier(ident),
          resolved_type: ExprType::Unknown,
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
      resolved_type: ExprType::Unknown,
    }
  }
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for PatternNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "pattern:{}-{} {:#?}", self.pos.0, self.pos.1, self.kind)
  }
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for PatternKind {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use PatternKind::*;

    match &self {
      Identifier(ident) => write!(f, "{:?}", ident),
      Constructor(ctor, arg_pattern) => write!(f, "constructor {:?} ({:#?})", ctor, arg_pattern),
      Tuple(elem_patterns) => write!(f, "tuple ({:#?})", elem_patterns),
      Record(elem_patterns) => write!(f, "record ({:#?})", elem_patterns),
      Underscore => write!(f, "wildcard"),
      Literal(lit) => write!(f, "literal ({:#?})", lit),
      Interpolation(parts) => write!(f, "interpolation ({:#?})", parts),
    }
  }
}
