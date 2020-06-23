use crate::tree_formatter::TreeFormatter;
use pluma_diagnostics::*;
use pluma_module::*;
use pluma_visitor::*;
use std::path::PathBuf;

pub struct Formatter<'a> {
  paths: &'a Vec<PathBuf>,
}

impl<'a> Formatter<'a> {
  pub fn new(paths: &'a Vec<PathBuf>) -> Self {
    Formatter { paths }
  }

  pub fn format(&self) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    for path in self.paths {
      if let Err(mut errs) = self.format_file(path) {
        diagnostics.append(&mut errs);
      }
    }

    if !diagnostics.is_empty() {
      return Err(diagnostics);
    }

    Ok(())
  }

  fn format_file(&self, path: &PathBuf) -> Result<(), Vec<Diagnostic>> {
    let mut module = Module::new("".to_owned(), path.into());

    module.parse()?;

    println!("AST: {:#?}", module.ast);
    let mut output = String::new();

    let mut tree_formatter = TreeFormatter::new(&mut output);
    module.traverse(&mut tree_formatter);

    println!("formatted:");
    println!("{}", output);

    Ok(())
  }
}

impl<'a> Visitor for Formatter<'a> {}
