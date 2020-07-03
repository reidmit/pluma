use crate::doc_generator::DocGenerator;
use pluma_compiler::*;
use pluma_visitor::*;

pub struct DocBuilder {
  compiler: Compiler,
}

impl DocBuilder {
  pub fn new(compiler: Compiler) -> Self {
    DocBuilder { compiler }
  }

  pub fn build(&mut self) {
    for (_module_name, module) in &mut self.compiler.modules {
      let mut comments = module.comments.as_ref().unwrap();
      let mut generator = DocGenerator::new(&mut comments);

      module.traverse(&mut generator);
    }
  }
}
