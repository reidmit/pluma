use std::collections::HashMap;
use crate::fs;
use crate::module::Module;
use crate::errors::{PackageCompilationError, ModuleCompilationError};
use crate::debug;

pub struct CompilerConfig {
  pub entry_path: Option<String>,
}

#[derive(Debug)]
pub struct Compiler {
  root_dir: String,
  entry_path: String,
  modules: HashMap<String, Result<Module, ModuleCompilationError>>,
}

impl Compiler {
  pub fn new(config: CompilerConfig) -> Result<Compiler, PackageCompilationError> {
    let (root_dir, entry_path) = fs::find_root_dir_and_entry_file(config.entry_path)
      .map_err(|err| PackageCompilationError::ConfigInvalid(err))?;

    Ok(Compiler {
      root_dir,
      entry_path,
      modules: HashMap::new(),
    })
  }

  pub fn run(&mut self) -> Result<(), PackageCompilationError> {
    self.modules.clear();

    let path = self.entry_path.to_string();
    self.compile_module(path);

    debug!("{:#?}", self);

    Ok(())
  }

  pub fn compile_module(&mut self, path: String) {
    let abs_path = fs::to_absolute_path(&path);
    let key = abs_path.clone();

    if self.modules.contains_key(&key) {
      return;
    }

    let mut module = Module::new(abs_path);
    let result = module.compile();

    match result {
      Ok(()) => {
        let imported_paths = module.get_referenced_paths();

        self.modules.insert(key, Ok(module));

        for imported_path in imported_paths {
          let module_path = fs::get_full_path_from_import(&self.root_dir, &imported_path);
          self.compile_module(module_path);
        }
      },
      Err(err) => {
        self.modules.insert(key, Err(err));
      },
    };
  }
}
