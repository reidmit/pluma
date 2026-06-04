use crate::location::Range;
use std::fmt;
use std::path::PathBuf;

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Diagnostic {
	pub kind: DiagnosticKind,
	// Stable identifier (e.g. `"E0103"`) for errors that carry one. Ad-hoc
	// diagnostics built from a bare `Display` (CLI/usage errors) leave this
	// `None`. The registry lives in `site/content/docs/diagnostics.md`; the
	// source of truth is each error kind's `code()` method.
	pub code: Option<&'static str>,
	// The primary one-line message.
	pub message: String,
	// The primary span the message points at.
	pub range: Option<Range>,
	// A single actionable suggestion (rendered as a `help:` line).
	pub help: Option<String>,
	// Secondary context lines (rendered as `note:` lines).
	pub notes: Vec<String>,
	// Secondary spans with their own captions (e.g. "previous definition here").
	pub labels: Vec<Label>,
	pub module_name: Option<String>,
	pub module_path: Option<PathBuf>,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum DiagnosticKind {
	Error,
	Warning,
}

impl DiagnosticKind {
	pub fn is_error(&self) -> bool {
		matches!(self, DiagnosticKind::Error)
	}
}

// A secondary span attached to a diagnostic, with its own caption. The
// diagnostic's primary `range` carries the main caret; labels add extra
// pointed-at locations (e.g. where a name was previously defined).
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Label {
	pub range: Range,
	pub message: String,
}

// Implemented by structured error kinds (parse + analysis). Lets each kind own
// its stable code, optional one-line `help`, and any longer `note` lines, while
// the primary message still comes from `Display`.
pub trait Reportable: fmt::Display {
	fn code(&self) -> &'static str;
	fn help(&self) -> Option<String> {
		None
	}
	fn notes(&self) -> Vec<String> {
		Vec::new()
	}
}

impl Diagnostic {
	// Ad-hoc error from anything `Display`. No code/help/notes — used by the CLI
	// and usage errors that aren't part of the structured frontend registry.
	pub fn error<E: fmt::Display>(err: E) -> Diagnostic {
		Diagnostic::bare(DiagnosticKind::Error, format!("{}", err))
	}

	pub fn warning<W: fmt::Display>(warning: W) -> Diagnostic {
		Diagnostic::bare(DiagnosticKind::Warning, format!("{}", warning))
	}

	// Structured error: pulls code/help/notes from the `Reportable`.
	pub fn report<R: Reportable>(r: R) -> Diagnostic {
		Diagnostic::from_reportable(DiagnosticKind::Error, r)
	}

	// Structured warning (e.g. unused binding).
	pub fn report_warning<R: Reportable>(r: R) -> Diagnostic {
		Diagnostic::from_reportable(DiagnosticKind::Warning, r)
	}

	fn from_reportable<R: Reportable>(kind: DiagnosticKind, r: R) -> Diagnostic {
		Diagnostic {
			kind,
			code: Some(r.code()),
			message: format!("{}", r),
			range: None,
			help: r.help(),
			notes: r.notes(),
			labels: Vec::new(),
			module_name: None,
			module_path: None,
		}
	}

	fn bare(kind: DiagnosticKind, message: String) -> Diagnostic {
		Diagnostic {
			kind,
			code: None,
			message,
			range: None,
			help: None,
			notes: Vec::new(),
			labels: Vec::new(),
			module_name: None,
			module_path: None,
		}
	}

	pub fn with_span(self, range: Range) -> Diagnostic {
		Diagnostic {
			range: Some(range),
			..self
		}
	}

	pub fn with_range(self, range: Range) -> Diagnostic {
		Diagnostic {
			range: Some(range),
			..self
		}
	}

	pub fn with_label(mut self, label: Label) -> Diagnostic {
		self.labels.push(label);
		self
	}

	pub fn with_module(self, module_name: String, module_path: PathBuf) -> Diagnostic {
		Diagnostic {
			module_name: Some(module_name),
			module_path: Some(module_path),
			..self
		}
	}

	pub fn is_error(&self) -> bool {
		self.kind.is_error()
	}
}
