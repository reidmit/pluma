use std::collections::HashMap;
use crate::fs;
use crate::ast::Node;
use crate::module::Module;
use crate::tokenizer::{Tokenizer, TokenizeResult};
use crate::errors::{PackageCompilationError, ModuleCompilationError};

const DEFAULT_ENTRY_FILE: &str = "main.pa";

pub struct CompilerConfig {
  pub entry_path: Option<String>,
}

pub struct Compiler {
  entry_path: String,
  modules: HashMap<String, Module>,
}

impl Compiler {
  pub fn new(config: CompilerConfig) -> Result<Compiler, PackageCompilationError> {
    let entry_path = fs::find_entry_file(config.entry_path)
      .map_err(|err| PackageCompilationError::ConfigInvalid(err))?;

    Ok(Compiler {
      entry_path,
      modules: HashMap::new()
    })
  }

  // TODO: make this support multiple modules (currently only entry)
  pub fn run(&mut self) -> Result<(), PackageCompilationError> {
    let path = self.entry_path.to_string();

    match self.compile_module(path) {
      Ok(()) => Ok(()),
      Err(err) => Err(PackageCompilationError::ModuleFailedToCompile(err))
    }
  }

  pub fn compile_module(&mut self, path: String) -> Result<(), ModuleCompilationError> {
    let key = path.to_string();
    let mut module = Module::new(path);
    let result = module.compile();
    self.modules.insert(key, module);
    result
  }
}
