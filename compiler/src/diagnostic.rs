use std::fmt;
use std::path::PathBuf;

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Diagnostic {
	pub kind: DiagnosticKind,
	pub message: String,
	pub span: Option<(usize, usize)>,
	pub module_name: Option<String>,
	pub module_path: Option<PathBuf>,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum DiagnosticKind {
	Error,
	Warning,
}

impl Diagnostic {
	pub fn error<E: fmt::Display>(err: E) -> Diagnostic {
		Diagnostic {
			kind: DiagnosticKind::Error,
			message: format!("{}", err),
			span: None,
			module_name: None,
			module_path: None,
		}
	}

	pub fn warning<W: fmt::Display>(warning: W) -> Diagnostic {
		Diagnostic {
			kind: DiagnosticKind::Warning,
			message: format!("{}", warning),
			span: None,
			module_name: None,
			module_path: None,
		}
	}

	pub fn with_span(self, span: (usize, usize)) -> Diagnostic {
		Diagnostic {
			span: Some(span),
			..self
		}
	}

	pub fn with_module(self, module_name: String, module_path: PathBuf) -> Diagnostic {
		Diagnostic {
			module_name: Some(module_name),
			module_path: Some(module_path),
			..self
		}
	}

	pub fn is_error(&self) -> bool {
		match &self.kind {
			DiagnosticKind::Error => true,
			_ => false,
		}
	}
}
