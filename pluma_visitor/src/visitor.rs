use pluma_ast::*;

pub trait Visitor {
  fn enter_call(&mut self, _node: &mut CallNode) {}

  fn leave_call(&mut self, _node: &mut CallNode) {}

  fn enter_const(&mut self, _node: &mut ConstNode) {}

  fn leave_const(&mut self, _node: &mut ConstNode) {}

  fn enter_def(&mut self, _node: &mut DefNode) {}

  fn leave_def(&mut self, _node: &mut DefNode) {}

  fn enter_expr(&mut self, _node: &mut ExprNode) {}

  fn leave_expr(&mut self, _node: &mut ExprNode) {}

  fn enter_identifier(&mut self, _node: &mut IdentifierNode) {}

  fn leave_identifier(&mut self, _node: &mut IdentifierNode) {}

  fn enter_intrinsic_def(&mut self, _node: &mut IntrinsicDefNode) {}

  fn leave_intrinsic_def(&mut self, _node: &mut IntrinsicDefNode) {}

  fn enter_intrinsic_type_def(&mut self, _node: &mut IntrinsicTypeDefNode) {}

  fn leave_intrinsic_type_def(&mut self, _node: &mut IntrinsicTypeDefNode) {}

  fn enter_let(&mut self, _node: &mut LetNode) {}

  fn leave_let(&mut self, _node: &mut LetNode) {}

  fn enter_literal(&mut self, _node: &mut LiteralNode) {}

  fn leave_literal(&mut self, _node: &mut LiteralNode) {}

  fn enter_match(&mut self, _node: &mut MatchNode) {}

  fn leave_match(&mut self, _node: &mut MatchNode) {}

  fn enter_match_case(&mut self, _node: &mut MatchCaseNode) {}

  fn leave_match_case(&mut self, _node: &mut MatchCaseNode) {}

  fn enter_module(&mut self, _node: &mut ModuleNode) {}

  fn leave_module(&mut self, _node: &mut ModuleNode) {}

  fn enter_operator(&mut self, _node: &mut OperatorNode) {}

  fn leave_operator(&mut self, _node: &mut OperatorNode) {}

  fn enter_pattern(&mut self, _node: &mut PatternNode) {}

  fn leave_pattern(&mut self, _node: &mut PatternNode) {}

  fn enter_return(&mut self, _node: &mut ReturnNode) {}

  fn leave_return(&mut self, _node: &mut ReturnNode) {}

  fn enter_statement(&mut self, _node: &mut StatementNode) {}

  fn leave_statement(&mut self, _node: &mut StatementNode) {}

  fn enter_top_level_statement(&mut self, _node: &mut TopLevelStatementNode) {}

  fn leave_top_level_statement(&mut self, _node: &mut TopLevelStatementNode) {}

  fn enter_type_expr(&mut self, _node: &mut TypeExprNode) {}

  fn leave_type_expr(&mut self, _node: &mut TypeExprNode) {}

  fn enter_type_def(&mut self, _node: &mut TypeDefNode) {}

  fn leave_type_def(&mut self, _node: &mut TypeDefNode) {}

  fn enter_type_identifier(&mut self, _node: &mut TypeIdentifierNode) {}

  fn leave_type_identifier(&mut self, _node: &mut TypeIdentifierNode) {}
}
