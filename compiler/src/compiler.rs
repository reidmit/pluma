use crate::diagnostics::Diagnostic;
use crate::module::Module;
use std::collections::HashMap;
use std::path::PathBuf;
use std::result;

pub type Result<T> = result::Result<T, Vec<Diagnostic>>;

#[derive(Debug)]
pub struct Compiler {
  pub root_dir: PathBuf,
  pub entry_module_name: String,
  pub modules: HashMap<String, Module>,
  diagnostics: Vec<Diagnostic>,
}

impl Compiler {
  pub fn from_dir(path: PathBuf) -> Result<Self> {
    if !path.is_dir() {
      todo!();
    }

    let entry_module_path = path.join("main.pa");
    if !entry_module_path.is_file() {
      todo!();
    }

    Ok(Compiler {
      root_dir: path.canonicalize().unwrap(),
      entry_module_name: "main".to_owned(),
      modules: HashMap::new(),
      diagnostics: Vec::new(),
    })
  }

  pub fn run(&mut self) -> Result<()> {
    self.parse_module(
      self.entry_module_name.clone(),
      self.to_module_path(self.entry_module_name.clone()),
    );

    if self.diagnostics.is_empty() {
      return Ok(());
    }

    Err(self.diagnostics.to_vec())
  }

  fn parse_module(&mut self, module_name: String, module_path: PathBuf) {
    if self.modules.contains_key(&module_name) {
      return;
    };

    let mut new_module = Module::new(module_name.clone(), module_path);
    let result = new_module.parse();
    let imported_module_names = new_module.get_referenced_module_names();
    self.modules.insert(module_name, new_module);

    if !result.is_ok() {
      self.diagnostics.append(&mut result.unwrap_err());
      return;
    }

    // println!("{:#?}", new_module);
    // println!("compiled: {:#?}", module_name);

    for imported_module_name in imported_module_names {
      self.parse_module(
        imported_module_name.clone(),
        self.to_module_path(imported_module_name),
      )
    }
  }

  fn to_module_path(&self, module_name: String) -> PathBuf {
    self.root_dir.join(module_name).with_extension("pa")
  }
}
