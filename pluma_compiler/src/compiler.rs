use crate::analyzer::Analyzer;
use crate::compiler_options::{CompilerMode, CompilerOptions};
use crate::dependency_graph::{DependencyGraph, TopologicalSort};
use crate::import_error::{ImportError, ImportErrorKind};
use crate::scope::Scope;
use crate::type_collector::TypeCollector;
use crate::usage_error::{UsageError, UsageErrorKind};
use pluma_constants::*;
use pluma_diagnostics::*;
use pluma_emitter::*;
use pluma_module::*;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Compiler {
  pub root_dir: PathBuf,
  pub entry_module_name: String,
  pub modules: HashMap<String, Module>,
  mode: CompilerMode,
  output_path: Option<String>,
  diagnostics: Vec<Diagnostic>,
  dependency_graph: DependencyGraph,
}

impl Compiler {
  /// Creates a new Compiler instance from the given options
  pub fn from_options(options: CompilerOptions) -> Result<Self, Vec<Diagnostic>> {
    let (root_dir, entry_module_name) = resolve_entry(options.entry_path)?;
    let dependency_graph = DependencyGraph::new(entry_module_name.clone());

    Ok(Compiler {
      root_dir,
      entry_module_name,
      modules: HashMap::new(),
      diagnostics: Vec::new(),
      output_path: options.output_path,
      mode: options.mode,
      dependency_graph,
    })
  }

  /// Parses input files without analyzing.
  pub fn parse(&mut self) -> Result<(), Vec<Diagnostic>> {
    self.parse_module(
      self.entry_module_name.clone(),
      to_module_path(self.root_dir.clone(), self.entry_module_name.clone()),
    );

    if !self.diagnostics.is_empty() {
      return Err(self.diagnostics.to_vec());
    }

    Ok(())
  }

  /// Checks input files without generating or emitting code (parses,
  /// resolves circular dependencies, type-checks).
  pub fn check(&mut self) -> Result<(), Vec<Diagnostic>> {
    self.parse()?;

    let sorted_names = match self.dependency_graph.sort() {
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

      let mut type_collector = TypeCollector::new(&mut module_scope);
      module_to_analyze.traverse(&mut type_collector);

      for diagnostic in type_collector.diagnostics {
        self.diagnostics.push(diagnostic.with_module(
          module_name.clone(),
          to_module_path(self.root_dir.clone(), self.entry_module_name.clone()),
        ))
      }

      let mut analyzer = Analyzer::new(&mut module_scope);
      module_to_analyze.traverse(&mut analyzer);

      for diagnostic in analyzer.diagnostics {
        self.diagnostics.push(diagnostic.with_module(
          module_name.clone(),
          to_module_path(self.root_dir.clone(), self.entry_module_name.clone()),
        ))
      }
    }

    if !self.diagnostics.is_empty() {
      return Err(self.diagnostics.to_vec());
    }

    Ok(())
  }

  /// Fully compiles input files & emits generated code
  pub fn emit(&mut self) -> Result<(), Vec<Diagnostic>> {
    self.check()?;

    let llvm_context = Emitter::create_context();
    let mut emitter = Emitter::new(&llvm_context);

    for module_name in self.sorted_module_names() {
      let module_to_emit = self.modules.get_mut(&module_name).unwrap();
      module_to_emit.traverse(&mut emitter);
    }

    if self.release_mode() {
      emitter.optimize();
    }

    if let Err(err) = emitter.verify() {
      self.diagnostics.push(err);
    }

    if let Some(path) = &self.output_path {
      if let Err(err) = emitter.write_to_path(Path::new(&path)) {
        self.diagnostics.push(err);
      }
    }

    if !self.diagnostics.is_empty() {
      return Err(self.diagnostics.to_vec());
    }

    Ok(())
  }

  /// Executes input without emitting any generated code
  pub fn run(&mut self) -> Result<i32, Vec<Diagnostic>> {
    self.check()?;

    let llvm_context = Emitter::create_context();
    let mut emitter = Emitter::new(&llvm_context);

    for module_name in self.sorted_module_names() {
      let module_to_emit = self.modules.get_mut(&module_name).unwrap();
      module_to_emit.traverse(&mut emitter);
    }

    if let Err(err) = emitter.verify() {
      self.diagnostics.push(err);
    }

    if !self.diagnostics.is_empty() {
      return Err(self.diagnostics.to_vec());
    }

    let exit_code = emitter.execute();

    return Ok(exit_code);
  }

  fn parse_module(&mut self, module_name: String, module_path: PathBuf) {
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
              to_module_path(self.root_dir.clone(), import_node.module_name.clone())
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

      self
        .dependency_graph
        .add_edge(import_node.module_name.clone(), module_name.clone());

      self.parse_module(
        import_node.module_name.clone(),
        to_module_path(self.root_dir.clone(), import_node.module_name),
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

    if Path::new(&to_module_path(self.root_dir.clone(), module_name.clone())).is_file() {
      return true;
    }

    return false;
  }

  fn release_mode(&self) -> bool {
    match self.mode {
      CompilerMode::Release => true,
      _ => false,
    }
  }

  fn sorted_module_names(&mut self) -> Vec<String> {
    match self.dependency_graph.sort() {
      TopologicalSort::Sorted(sorted_names) => sorted_names.to_vec(),
      _ => unreachable!(),
    }
  }
}

fn resolve_entry(entry_path: String) -> Result<(PathBuf, String), Vec<Diagnostic>> {
  match get_root_dir_and_module_name(entry_path) {
    Ok(result) => Ok(result),
    Err(usage_error) => Err(vec![Diagnostic::error(usage_error)]),
  }
}

fn to_module_path(root_dir: PathBuf, module_name: String) -> PathBuf {
  root_dir.join(module_name).with_extension("pa")
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
