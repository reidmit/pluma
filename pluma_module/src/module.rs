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
  pub bytes: Option<Vec<u8>>,
  pub ast: Option<ModuleNode>,
  imports: Option<Vec<UseNode>>,
}

impl Module {
  pub fn new(module_name: String, module_path: PathBuf) -> Module {
    Module {
      module_name,
      module_path,
      bytes: None,
      ast: None,
      imports: None,
    }
  }

  pub fn parse(&mut self) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    if !self.read(&mut diagnostics) {
      return Err(diagnostics);
    }

    self.build_ast(&mut diagnostics);

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

  pub fn traverse<V: Visitor>(&mut self, visitor: &mut V) {
    if let Some(ast) = &mut self.ast {
      ast.traverse(visitor)
    }
  }

  fn read(&mut self, diagnostics: &mut Vec<Diagnostic>) -> bool {
    match fs::read(&self.module_path) {
      Ok(bytes) => {
        self.bytes = Some(bytes);
        true
      }

      Err(err) => {
        diagnostics.push(
          Diagnostic::error(err)
            .with_module(self.module_name.clone(), self.module_path.to_path_buf()),
        );

        false
      }
    }
  }

  fn build_ast(&mut self, diagnostics: &mut Vec<Diagnostic>) {
    let bytes = self.bytes.as_ref().unwrap();
    let tokenizer = Tokenizer::from_source(&bytes);

    let (ast, imports, errors) =
      Parser::new(self.bytes.as_ref().unwrap(), tokenizer).parse_module();

    if errors.is_empty() {
      self.ast = Some(ast);
      self.imports = Some(imports);
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
