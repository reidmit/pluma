use crate::item_collector::ItemCollector;
use pluma_compiler::*;

pub struct DocBuilder {
  compiler: Compiler,
}

impl DocBuilder {
  pub fn new(compiler: Compiler) -> Self {
    DocBuilder { compiler }
  }

  pub fn build(&mut self) {
    for (_module_name, module) in &mut self.compiler.modules {
      let comments = module.comments.as_ref().unwrap();
      let line_break_positions = module.line_break_positions.as_ref().unwrap();

      let mut item_collector = ItemCollector::new(comments, line_break_positions);
      module.traverse(&mut item_collector);

      println!("items: {:#?}", item_collector.items);
    }
  }
}
