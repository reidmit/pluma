pub struct ModuleCompiler {
  source: String,
}

pub struct VM {
  modules: Vec<ModuleCompiler>
}

impl VM {
  add_module(&mut self, module: ModuleCompiler) {
    self.modules.add(module);
  }
}