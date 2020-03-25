use crate::dependency_graph::{DependencyGraph, TopologicalSort};
use crate::diagnostics::Diagnostic;
use crate::import_error::{ImportError, ImportErrorKind};
use crate::module::Module;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    let mut dependency_graph = DependencyGraph::new(self.entry_module_name.clone());

    self.parse_module(
      self.entry_module_name.clone(),
      self.to_module_path(self.entry_module_name.clone()),
      &mut dependency_graph,
    );

    if !self.diagnostics.is_empty() {
      return Err(self.diagnostics.to_vec());
    }

    let sorted_names = match dependency_graph.sort() {
      TopologicalSort::Cycle(names) => {
        self.diagnostics.push(Diagnostic::error(ImportError {
          kind: ImportErrorKind::CyclicalDependency(names.to_vec()),
        }));

        return Err(self.diagnostics.to_vec());
      }
      TopologicalSort::Sorted(names) => names,
    };

    println!("sorted: {:#?}", sorted_names);

    Ok(())
  }

  fn parse_module(
    &mut self,
    module_name: String,
    module_path: PathBuf,
    dependency_graph: &mut DependencyGraph,
  ) {
    if self.modules.contains_key(&module_name) {
      return;
    };

    let mut new_module = Module::new(module_name.clone(), module_path.to_owned());
    let result = new_module.parse();
    let imports = new_module.get_imports();
    self.modules.insert(module_name.clone(), new_module);

    for import_node in imports {
      let imported_module_name = import_node.module_name.clone();

      if !self.module_name_exists(imported_module_name) {
        self.diagnostics.push(
          Diagnostic::error(ImportError {
            kind: ImportErrorKind::ModuleNotFound(
              import_node.module_name.clone(),
              self
                .to_module_path(import_node.module_name.clone())
                .to_str()
                .unwrap()
                .to_owned(),
            ),
          })
          .with_module(module_name.clone(), module_path.to_owned())
          .with_pos(import_node.pos),
        );

        continue;
      }

      dependency_graph.add_edge(import_node.module_name.clone(), module_name.clone());

      self.parse_module(
        import_node.module_name.clone(),
        self.to_module_path(import_node.module_name),
        dependency_graph,
      )
    }

    if !result.is_ok() {
      self.diagnostics.append(&mut result.unwrap_err());
      return;
    }
  }

  fn module_name_exists(&self, module_name: String) -> bool {
    if self.modules.contains_key(&module_name) {
      return true;
    }

    if Path::new(&self.to_module_path(module_name.clone())).is_file() {
      return true;
    }

    return false;
  }

  fn to_module_path(&self, module_name: String) -> PathBuf {
    self.root_dir.join(module_name).with_extension("pa")
  }
}
