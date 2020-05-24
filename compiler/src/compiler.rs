use crate::analyzer::Analyzer;
use crate::dependency_graph::{DependencyGraph, TopologicalSort};
use crate::diagnostics::Diagnostic;
use crate::import_error::{ImportError, ImportErrorKind};
use crate::module::Module;
use crate::scope::Scope;
use crate::type_collector::TypeCollector;
use crate::usage_error::{UsageError, UsageErrorKind};
use crate::{DEFAULT_ENTRY_MODULE_NAME, FILE_EXTENSION};
use std::collections::HashMap;
use std::env;
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
  pub fn from_path(entry_path: String) -> Result<Self> {
    let (root_dir, entry_module_name) = Compiler::resolve_entry(entry_path)?;

    Ok(Compiler {
      root_dir,
      entry_module_name,
      modules: HashMap::new(),
      diagnostics: Vec::new(),
    })
  }

  fn resolve_entry(entry_path: String) -> Result<(PathBuf, String)> {
    match get_root_dir_and_module_name(entry_path) {
      Ok(result) => Ok(result),
      Err(usage_error) => Err(vec![Diagnostic::error(usage_error)]),
    }
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

    for module_name in sorted_names {
      let mut module_scope = Scope::new();

      module_scope.enter();

      let module_to_analyze = self.modules.get_mut(module_name).unwrap();

      println!("{:#?}", module_to_analyze.ast);

      let mut type_collector = TypeCollector::new(&mut module_scope);
      module_to_analyze.traverse(&mut type_collector);

      let mut analyzer = Analyzer::new(&mut module_scope);
      module_to_analyze.traverse(&mut analyzer);

      for diagnostic in analyzer.diagnostics {
        self.diagnostics.push(diagnostic.with_module(
          module_name.clone(),
          self.to_module_path(self.entry_module_name.clone()),
        ))
      }
    }

    if !self.diagnostics.is_empty() {
      return Err(self.diagnostics.to_vec());
    }

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

fn get_root_dir_and_module_name(
  entry_path: String,
) -> std::result::Result<(PathBuf, String), UsageError> {
  let mut joined_path = Path::new(&env::current_dir().unwrap()).join(entry_path);
  let mut found_dir = false;

  if !joined_path.exists() {
    joined_path.set_extension(FILE_EXTENSION);
  } else if joined_path.is_dir() {
    found_dir = true;
    joined_path.push(DEFAULT_ENTRY_MODULE_NAME);
    joined_path.set_extension(FILE_EXTENSION);
  }

  match joined_path.canonicalize() {
    Ok(abs_path) => Ok((
      abs_path.parent().unwrap().to_path_buf(),
      abs_path.file_stem().unwrap().to_str().unwrap().to_owned(),
    )),

    Err(_) => {
      if found_dir {
        return Err(UsageError {
          kind: UsageErrorKind::EntryDirDoesNotContainEntryFile(
            joined_path.parent().unwrap().to_str().unwrap().to_owned(),
          ),
        });
      }

      Err(UsageError {
        kind: UsageErrorKind::InvalidEntryPath(joined_path.to_str().unwrap().to_owned()),
      })
    }
  }
}
