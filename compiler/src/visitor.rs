use crate::ast::*;

pub trait Visitor {
  fn enter_def(&mut self, node: &DefNode) {}

  fn leave_def(&mut self, node: &DefNode) {}

  fn enter_expr(&mut self, node: &ExprNode) {}

  fn leave_expr(&mut self, node: &ExprNode) {}

  fn enter_identifier(&mut self, node: &IdentifierNode) {}

  fn leave_identifier(&mut self, node: &IdentifierNode) {}

  fn enter_let(&mut self, node: &LetNode) {}

  fn leave_let(&mut self, node: &LetNode) {}

  fn enter_literal(&mut self, node: &LiteralNode) {}

  fn leave_literal(&mut self, node: &LiteralNode) {}

  fn enter_match(&mut self, node: &MatchNode) {}

  fn leave_match(&mut self, node: &MatchNode) {}

  fn enter_match_case(&mut self, node: &MatchCaseNode) {}

  fn leave_match_case(&mut self, node: &MatchCaseNode) {}

  fn enter_module(&mut self, node: &ModuleNode) {}

  fn leave_module(&mut self, node: &ModuleNode) {}

  fn enter_operator(&mut self, node: &OperatorNode) {}

  fn leave_operator(&mut self, node: &OperatorNode) {}

  fn enter_pattern(&mut self, node: &PatternNode) {}

  fn leave_pattern(&mut self, node: &PatternNode) {}

  fn enter_return(&mut self, node: &ReturnNode) {}

  fn leave_return(&mut self, node: &ReturnNode) {}

  fn enter_statement(&mut self, node: &StatementNode) {}

  fn leave_statement(&mut self, node: &StatementNode) {}

  fn enter_top_level_statement(&mut self, node: &TopLevelStatementNode) {}

  fn leave_top_level_statement(&mut self, node: &TopLevelStatementNode) {}

  fn enter_type(&mut self, node: &TypeNode) {}

  fn leave_type(&mut self, node: &TypeNode) {}

  fn enter_type_def(&mut self, node: &TypeDefNode) {}

  fn leave_type_def(&mut self, node: &TypeDefNode) {}
}
