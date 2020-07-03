use pluma_compiler::*;

pub struct DocGenerator {
  compiler: Compiler,
}

impl DocGenerator {
  pub fn new(compiler: Compiler) -> Self {
    DocGenerator { compiler }
  }

  pub fn generate(&self) {
    println!("{:#?}", self.compiler.modules);
  }
}
