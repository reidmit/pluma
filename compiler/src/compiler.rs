use crate::dependency_graph::{DependencyGraph, TopologicalSort};
use crate::errors::PackageCompilationError;
use crate::fs;
use crate::module::Module;
use std::collections::HashMap;

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
  dependency_graph: DependencyGraph,
}

impl Compiler {
  pub fn new(config: CompilerConfig) -> Compiler {
    let dependency_graph = DependencyGraph::new(config.entry_module_name.to_string());

    Compiler {
      root_dir: config.root_dir,
      entry_module_name: config.entry_module_name,
      modules: HashMap::new(),
      dependency_graph,
    }
  }

  pub fn run(&mut self) -> Result<(), PackageCompilationError> {
    self.modules.clear();

    let entry_module_name = &self.entry_module_name.to_string();
    self.parse_module(entry_module_name.to_string())?;
    self.check_for_module_errors()?;

    let sorted_modules = match self.dependency_graph.sort() {
      TopologicalSort::Cycle(cycle) => {
        return Err(PackageCompilationError::CyclicalDependency(
          cycle.to_owned(),
        ));
      }
      TopologicalSort::Sorted(sorted_modules) => sorted_modules.clone(),
    };

    for module_name in sorted_modules {
      self.analyze_module(module_name.to_string())?;
    }

    self.check_for_module_errors()?;

    Ok(())
  }

  pub fn parse_module(&mut self, module_name: String) -> Result<(), PackageCompilationError> {
    if self.modules.contains_key(&module_name) {
      return Ok(());
    }

    let module_path = fs::to_absolute_path(&self.root_dir, &module_name);
    let get_module_name = || module_name.clone();

    let mut module = Module::new(get_module_name(), module_path);
    module.read_and_parse();

    let referenced = module.get_referenced_paths();
    self.modules.insert(get_module_name(), module);

    match referenced {
      Some(imported_module_names) => {
        for imported_module_name in imported_module_names {
          self
            .dependency_graph
            .add_edge(imported_module_name.to_owned(), get_module_name());

          self.parse_module(imported_module_name)?;
        }
      }
      None => {}
    }

    Ok(())
  }

  pub fn analyze_module(&mut self, _module_name: String) -> Result<(), PackageCompilationError> {
    // let module = self.modules.get_mut(&module_name).unwrap();
    // module.analyze();
    Ok(())
  }

  fn check_for_module_errors(&self) -> Result<(), PackageCompilationError> {
    let mut modules_with_errors = Vec::new();

    for (module_path, module) in &self.modules {
      if module.has_errors() {
        modules_with_errors.push(module_path.clone());
      }
    }

    if !modules_with_errors.is_empty() {
      return Err(PackageCompilationError::ModulesFailedToCompile(
        modules_with_errors,
      ));
    }

    Ok(())
  }
}
