use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Diagnostic {
  pub kind: DiagnosticKind,
  pub message: String,
  pub pos: Option<(usize, usize)>,
  pub module_name: Option<String>,
  pub module_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum DiagnosticKind {
  Error,
}

impl Diagnostic {
  pub fn error<E: fmt::Display>(err: E) -> Diagnostic {
    Diagnostic {
      kind: DiagnosticKind::Error,
      message: format!("{}", err),
      pos: None,
      module_name: None,
      module_path: None,
    }
  }

  pub fn with_pos(self, pos: (usize, usize)) -> Diagnostic {
    Diagnostic {
      kind: self.kind,
      message: self.message,
      pos: Some(pos),
      module_name: self.module_name,
      module_path: self.module_path,
    }
  }

  pub fn with_module(self, module_name: String, module_path: PathBuf) -> Diagnostic {
    Diagnostic {
      kind: self.kind,
      message: self.message,
      pos: self.pos,
      module_name: Some(module_name),
      module_path: Some(module_path),
    }
  }
}
