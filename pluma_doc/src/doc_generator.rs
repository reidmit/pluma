use pluma_ast::*;
use pluma_parser::*;
use pluma_visitor::*;

pub struct DocGenerator<'a> {
  comments: &'a CommentMap,
}

impl<'a> DocGenerator<'a> {
  pub fn new(comments: &'a CommentMap) -> Self {
    DocGenerator { comments }
  }
}

impl<'a> Visitor for DocGenerator<'a> {
  fn enter_module(&mut self, _node: &ModuleNode) {
  }

  fn enter_top_level_statement(&mut self, node: &TopLevelStatementNode) {
    match &node.kind {
      TopLevelStatementKind::Def(def) => match &def.kind {
        DefKind::Function { signature } => {
          println!("{:#?}", signature);
        }

        _ => {}
      },

      _ => {}
    }
  }
}
