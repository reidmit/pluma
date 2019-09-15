use std::collections::HashMap;
use std::path::Path;
use crate::fs;
use crate::module::Module;
use crate::errors::{PackageCompilationError};
use crate::debug;
use crate::import_chain::ImportChain;

pub struct CompilerConfig {
  // Absolute path to the package root directory
  pub root_dir: String,
  // Absolute path to the package entry file (main.pa)
  pub entry_path: String,
}

#[derive(Debug)]
pub struct Compiler {
  pub root_dir: String,
  pub entry_path: String,
  pub modules: HashMap<String, Module>,
}

impl Compiler {
  pub fn new(config: CompilerConfig) -> Compiler {
    Compiler {
      root_dir: config.root_dir,
      entry_path: config.entry_path,
      modules: HashMap::new(),
    }
  }

  pub fn run(&mut self) -> Result<(), PackageCompilationError> {
    self.modules.clear();

    let path = Path::new(&self.entry_path)
      .strip_prefix(&self.root_dir)
      .unwrap()
      .to_str()
      .unwrap()
      .to_owned();

    let result = self.compile_module(path, ImportChain::new());

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

  pub fn compile_module(&mut self, path: String, import_chain: ImportChain) -> Result<(), PackageCompilationError> {
    let abs_path = fs::to_absolute_path(&self.root_dir, &path);
    let get_path = || path.clone();
    let key = get_path();

    if self.modules.contains_key(&key) {
      if import_chain.contains(get_path()) {
        let mut chain = import_chain.entries;
        chain.push(get_path());
        return Err(PackageCompilationError::CyclicalDependency(chain));
      }

      return Ok(());
    }

    let mut module = Module::new(abs_path, get_path());
    module.compile();

    let referenced = module.get_referenced_paths();
    self.modules.insert(key, module);

    match referenced {
      Some(imported_paths) => {
        for imported_path in imported_paths {
          let module_path = fs::get_full_path_from_import(&self.root_dir, &imported_path);
          let mut new_import_chain = import_chain.clone();
          new_import_chain.add(get_path());

          self.compile_module(module_path, new_import_chain)?;
        }
      },
      None => {}
    }

    Ok(())
  }
}
