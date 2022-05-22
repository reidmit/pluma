use super::*;
use crate::typing::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct PatternNode {
  pub span: Span,
  pub kind: PatternKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
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
    let span = self.span;

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
          span: ident.span,
          kind: ExprKind::Identifier(ident),
          ty: Type::Unknown,
        };

        let arg_expr = arg.to_expr();

        let call = CallNode {
          span,
          callee: Box::new(callee),
          args: vec![arg_expr],
        };

        ExprKind::Call(call)
      }

      _other => todo!("other expr kind in pattern"),
    };

    ExprNode {
      span,
      kind: expr_kind,
      ty: Type::Unknown,
    }
  }
}
