use std::fmt;

#[derive(Debug, Clone)]
pub struct ImportError {
  pub kind: ImportErrorKind,
}

#[derive(Debug, Clone)]
pub enum ImportErrorKind {
  CyclicalDependency(Vec<String>),
  ModuleNotFound(String, String),
}

impl fmt::Display for ImportError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use ImportErrorKind::*;

    match &self.kind {
      CyclicalDependency(module_names) => write!(
        f,
        "Import cycle between modules: '{}'",
        module_names.join("' -> '")
      ),
      ModuleNotFound(module_name, module_path) => write!(
        f,
        "Module '{}' not found.\n\nExpected to find it at '{}'.",
        module_name, module_path,
      ),
    }
  }
}
