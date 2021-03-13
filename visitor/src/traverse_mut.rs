use crate::visitor_mut::VisitorMut;
use ast::*;

pub trait TraverseMut {
  fn traverse_mut<V: VisitorMut>(&mut self, _visitor: &mut V) {}
}

impl TraverseMut for BlockNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_block(self);

    if let Some(param) = &mut self.param {
      param.traverse_mut(visitor);
    }

    for stmt in &mut self.body {
      stmt.traverse_mut(visitor);
    }

    visitor.leave_block(self);
  }
}

impl TraverseMut for CallNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_call(self);

    // for arg in &mut self.args {
    //   arg.traverse_mut(visitor);
    // }

    // self.callee.traverse_mut(visitor);

    visitor.leave_call(self);
  }
}

impl TraverseMut for DefNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_def(self);

    match &mut self.kind {
      DefKind::Function { signature } => {
        for (ident, type_expr) in signature {
          ident.traverse_mut(visitor);
          type_expr.traverse_mut(visitor);
        }
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        for (ident, type_expr) in signature {
          ident.traverse_mut(visitor);
          type_expr.traverse_mut(visitor);
        }

        receiver.traverse_mut(visitor);
      }
    }

    self.block.traverse_mut(visitor);

    visitor.leave_def(self);
  }
}

impl TraverseMut for ExprNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_expr(self);

    match &mut self.kind {
      ExprKind::Access { receiver, property } => {
        receiver.traverse_mut(visitor);
        property.traverse_mut(visitor);
      }

      ExprKind::Assignment { left, right } => {
        right.traverse_mut(visitor);
        left.traverse_mut(visitor);
      }

      ExprKind::BinaryOperation { op, left, right } => {
        op.traverse_mut(visitor);
        left.traverse_mut(visitor);
        right.traverse_mut(visitor);
      }

      ExprKind::Block { block } => block.traverse_mut(visitor),

      ExprKind::Call { call } => call.traverse_mut(visitor),

      ExprKind::EmptyTuple => {}

      ExprKind::Literal { literal } => literal.traverse_mut(visitor),

      ExprKind::Grouping { inner } => inner.traverse_mut(visitor),

      ExprKind::Identifier { ident } => ident.traverse_mut(visitor),

      ExprKind::Match { match_ } => match_.traverse_mut(visitor),

      ExprKind::UnaryOperation { op, right } => {
        op.traverse_mut(visitor);
        right.traverse_mut(visitor);
      }

      ExprKind::Interpolation { parts } => {
        for part in parts {
          part.traverse_mut(visitor);
        }
      }

      ExprKind::MultiPartIdentifier { parts } => {
        for part in parts {
          part.traverse_mut(visitor);
        }
      }

      ExprKind::Tuple { entries } => {
        for (label, entry) in entries {
          if let Some(ident) = label {
            ident.traverse_mut(visitor);
          }

          entry.traverse_mut(visitor);
        }
      }

      ExprKind::TypeAssertion {
        expr,
        asserted_type,
      } => {
        expr.traverse_mut(visitor);
        asserted_type.traverse_mut(visitor);
      }

      _other_kind => todo!("traverseMut other kind"),
    }

    visitor.leave_expr(self);
  }
}

impl TraverseMut for IdentifierNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_identifier(self);

    visitor.leave_identifier(self);
  }
}

impl TraverseMut for IntrinsicDefNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_intrinsic_def(self);

    visitor.leave_intrinsic_def(self);
  }
}

impl TraverseMut for IntrinsicTypeDefNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_intrinsic_type_def(self);

    visitor.leave_intrinsic_type_def(self);
  }
}

impl TraverseMut for LetNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_let(self);

    self.pattern.traverse_mut(visitor);
    self.value.traverse_mut(visitor);

    visitor.leave_let(self);
  }
}

impl TraverseMut for LiteralNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_literal(self);

    visitor.leave_literal(self);
  }
}

impl TraverseMut for MatchNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_match(self);

    self.subject.traverse_mut(visitor);

    for case in &mut self.cases {
      case.traverse_mut(visitor);
    }

    visitor.leave_match(self);
  }
}

impl TraverseMut for MatchCaseNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_match_case(self);

    self.pattern.traverse_mut(visitor);
    self.body.traverse_mut(visitor);

    visitor.leave_match_case(self);
  }
}

impl TraverseMut for ModuleNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_module(self);

    for node in &mut self.body {
      node.traverse_mut(visitor);
    }

    visitor.leave_module(self);
  }
}

impl TraverseMut for OperatorNode {
  // todo
}

impl TraverseMut for PatternNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_pattern(self);

    // ?

    visitor.leave_pattern(self);
  }
}

impl TraverseMut for StatementNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_statement(self);

    match &mut self.kind {
      StatementKind::Let(node) => node.traverse_mut(visitor),
      StatementKind::Expr(node) => node.traverse_mut(visitor),
    };

    visitor.leave_statement(self);
  }
}

impl TraverseMut for TopLevelStatementNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_top_level_statement(self);

    match &mut self.kind {
      TopLevelStatementKind::Let(node) => node.traverse_mut(visitor),
      TopLevelStatementKind::TypeDef(node) => node.traverse_mut(visitor),
      TopLevelStatementKind::Def(node) => node.traverse_mut(visitor),
      TopLevelStatementKind::Expr(node) => node.traverse_mut(visitor),
      TopLevelStatementKind::IntrinsicDef(node) => node.traverse_mut(visitor),
      TopLevelStatementKind::IntrinsicTypeDef(node) => node.traverse_mut(visitor),
      TopLevelStatementKind::VisibilityMarker(..) => {}
    };

    visitor.leave_top_level_statement(self);
  }
}

impl TraverseMut for TypeExprNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_type_expr(self);

    visitor.leave_type_expr(self);
  }
}

impl TraverseMut for TypeDefNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_type_def(self);

    visitor.leave_type_def(self);
  }
}

impl TraverseMut for TypeIdentifierNode {
  fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    visitor.enter_type_identifier(self);

    visitor.leave_type_identifier(self);
  }
}
