use crate::ast::*;

pub trait VisitorMut {
  fn enter_def(&mut self, node: &mut DefNode) {}

  fn leave_def(&mut self, node: &mut DefNode) {}

  fn enter_expr(&mut self, node: &mut ExprNode) {}

  fn leave_expr(&mut self, node: &mut ExprNode) {}

  fn enter_identifier(&mut self, node: &mut IdentifierNode) {}

  fn leave_identifier(&mut self, node: &mut IdentifierNode) {}

  fn enter_let(&mut self, node: &mut LetNode) {}

  fn leave_let(&mut self, node: &mut LetNode) {}

  fn enter_literal(&mut self, node: &mut LiteralNode) {}

  fn leave_literal(&mut self, node: &mut LiteralNode) {}

  fn enter_match(&mut self, node: &mut MatchNode) {}

  fn leave_match(&mut self, node: &mut MatchNode) {}

  fn enter_match_case(&mut self, node: &mut MatchCaseNode) {}

  fn leave_match_case(&mut self, node: &mut MatchCaseNode) {}

  fn enter_module(&mut self, node: &mut ModuleNode) {}

  fn leave_module(&mut self, node: &mut ModuleNode) {}

  fn enter_operator(&mut self, node: &mut OperatorNode) {}

  fn leave_operator(&mut self, node: &mut OperatorNode) {}

  fn enter_pattern(&mut self, node: &mut PatternNode) {}

  fn leave_pattern(&mut self, node: &mut PatternNode) {}

  fn enter_return(&mut self, node: &mut ReturnNode) {}

  fn leave_return(&mut self, node: &mut ReturnNode) {}

  fn enter_statement(&mut self, node: &mut StatementNode) {}

  fn leave_statement(&mut self, node: &mut StatementNode) {}

  fn enter_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {}

  fn leave_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {}

  fn enter_type(&mut self, node: &mut TypeNode) {}

  fn leave_type(&mut self, node: &mut TypeNode) {}

  fn enter_type_def(&mut self, node: &mut TypeDefNode) {}

  fn leave_type_def(&mut self, node: &mut TypeDefNode) {}
}
