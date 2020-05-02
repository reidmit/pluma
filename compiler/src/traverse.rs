use crate::ast::*;
use crate::visitor::Visitor;

pub trait Traverse {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {}
}

impl Traverse for DefNode {
  // todo
}

impl Traverse for ExprNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_expr(self);

    self.kind.traverse(visitor);

    visitor.leave_expr(self);
  }
}

impl Traverse for ExprKind {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    match &self {
      ExprKind::Literal(node) => node.traverse(visitor),
      ExprKind::Identifier(node) => node.traverse(visitor),
      _ => todo!(),
    }
  }
}

impl Traverse for IdentifierNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_identifier(self);

    visitor.leave_identifier(self);
  }
}

impl Traverse for LetNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_let(self);

    self.pattern.traverse(visitor);
    self.value.traverse(visitor);

    visitor.leave_let(self);
  }
}

impl Traverse for LiteralNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_literal(self);

    visitor.leave_literal(self);
  }
}

impl Traverse for MatchNode {
  // todo
}

impl Traverse for MatchCaseNode {
  // todo
}

impl Traverse for ModuleNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_module(&self);

    for node in &self.body {
      node.traverse(visitor);
    }

    visitor.leave_module(&self);
  }
}

impl Traverse for OperatorNode {
  // todo
}

impl Traverse for PatternNode {
  // todo
}

impl Traverse for ReturnNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_return(self);

    self.value.traverse(visitor);

    visitor.leave_return(self);
  }
}

impl Traverse for StatementNode {}

impl Traverse for TopLevelStatementNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    visitor.enter_top_level_statement(self);

    match &self.kind {
      TopLevelStatementKind::Let(node) => node.traverse(visitor),
      TopLevelStatementKind::TypeDef(node) => node.traverse(visitor),
      TopLevelStatementKind::Def(node) => node.traverse(visitor),
      TopLevelStatementKind::Expr(node) => node.traverse(visitor),
    };

    visitor.leave_top_level_statement(self);
  }
}

impl Traverse for TypeNode {
  // todo
}

impl Traverse for TypeDefNode {
  fn traverse<V: Visitor>(&self, visitor: &mut V) {
    // match &self.kind {
    //   TypeDefKind::Enum { variants } => {
    //     for variant in variants {
    //       visitor.visit_type(&variant);
    //     }
    //   }
    //   _ => todo!("not yet implemented"),
    // }
  }
}
