use std::collections::HashMap;
use crate::fs;
use crate::module::Module;
use crate::errors::{PackageCompilationError};
use crate::debug;
use crate::import_chain::ImportChain;

pub struct CompilerConfig {
  // Absolute path to the package root directory
  pub root_dir: String,
  // Entry module name (e.g. "main")
  pub entry_module_name: String,
}

#[derive(Debug)]
pub struct Compiler {
  pub root_dir: String,
  pub entry_module_name: String,
  pub modules: HashMap<String, Module>,
}

impl Compiler {
  pub fn new(config: CompilerConfig) -> Compiler {
    Compiler {
      root_dir: config.root_dir,
      entry_module_name: config.entry_module_name,
      modules: HashMap::new(),
    }
  }

  pub fn run(&mut self) -> Result<(), PackageCompilationError> {
    self.modules.clear();

    let module_name = &self.entry_module_name.to_string();
    let result = self.compile_module(module_name.to_string(), ImportChain::new());

    debug!("{:#?}", self);

    match result {
      Ok(()) => {
        let mut modules_with_errors = Vec::new();

        for (module_path, module) in &self.modules {
          if module.has_errors() {
            modules_with_errors.push(module_path.clone());
          }
        }

        match modules_with_errors.is_empty() {
          true => Ok(()),
          _ => Err(PackageCompilationError::ModulesFailedToCompile(modules_with_errors))
        }
      }
      err => err,
    }
  }

  pub fn compile_module(&mut self, module_name: String, import_chain: ImportChain) -> Result<(), PackageCompilationError> {
    let module_path = fs::to_absolute_path(&self.root_dir, &module_name);
    let get_module_name = || module_name.clone();
    let key = get_module_name();

    if self.modules.contains_key(&key) {
      if import_chain.contains(get_module_name()) {
        let mut chain = import_chain.entries;
        chain.push(get_module_name());
        return Err(PackageCompilationError::CyclicalDependency(chain));
      }

      return Ok(());
    }

    let mut module = Module::new(get_module_name(), module_path);
    module.compile();

    let referenced = module.get_referenced_paths();
    self.modules.insert(key, module);

    match referenced {
      Some(imported_module_names) => {
        for imported_module_name in imported_module_names {
          let mut new_import_chain = import_chain.clone();
          new_import_chain.add(get_module_name());
          self.compile_module(imported_module_name, new_import_chain)?;
        }
      },
      None => {}
    }

    Ok(())
  }
}
