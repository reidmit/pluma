use pluma_ast::*;
use pluma_diagnostics::*;
use pluma_parser::*;
use pluma_visitor::*;
use std::fs;
use std::path::PathBuf;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Module {
  pub module_name: String,
  pub module_path: PathBuf,
  pub ast: Option<ModuleNode>,
  pub comments: Option<CommentMap>,
  imports: Option<Vec<UseNode>>,
  collect_comments: bool,
}

impl Module {
  pub fn new(module_name: String, module_path: PathBuf, collect_comments: bool) -> Module {
    Module {
      module_name,
      module_path,
      ast: None,
      imports: None,
      comments: None,
      collect_comments,
    }
  }

  pub fn parse(&mut self) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    match fs::read(&self.module_path) {
      Ok(bytes) => self.build_ast(bytes, &mut diagnostics),
      Err(err) => diagnostics.push(
        Diagnostic::error(err)
          .with_module(self.module_name.clone(), self.module_path.to_path_buf()),
      ),
    }

    if diagnostics.is_empty() {
      return Ok(());
    }

    return Err(diagnostics);
  }

  pub fn did_parse(&self) -> bool {
    self.ast.is_some()
  }

  pub fn get_imports(&self) -> Vec<UseNode> {
    let mut imports = Vec::new();

    if self.imports.is_none() {
      return imports;
    }

    for import_node in self.imports.as_ref().unwrap() {
      imports.push(import_node.clone())
    }

    imports
  }

  pub fn traverse<V: Visitor>(&self, visitor: &mut V) {
    if let Some(ast) = &self.ast {
      ast.traverse(visitor)
    }
  }

  pub fn traverse_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
    if let Some(ast) = &mut self.ast {
      ast.traverse_mut(visitor)
    }
  }

  fn build_ast(&mut self, bytes: Vec<u8>, diagnostics: &mut Vec<Diagnostic>) {
    let tokenizer = Tokenizer::from_source(&bytes, self.collect_comments);

    let (ast, imports, comments, errors) =
      Parser::new(&bytes, tokenizer, self.collect_comments).parse_module();

    if errors.is_empty() {
      self.ast = Some(ast);
      self.imports = Some(imports);
      self.comments = comments;
      return;
    }

    for err in errors {
      diagnostics.push(
        Diagnostic::error(err)
          .with_pos(err.pos)
          .with_module(self.module_name.clone(), self.module_path.to_path_buf()),
      );
    }
  }
}
