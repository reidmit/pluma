use crate::colors;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Diagnostic {
	pub kind: DiagnosticKind,
	pub message: String,
	pub loc: Option<(usize, usize)>,
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
			loc: None,
			module_name: None,
			module_path: None,
		}
	}

	pub fn warning<W: fmt::Display>(warning: W) -> Diagnostic {
		Diagnostic {
			kind: DiagnosticKind::Warning,
			message: format!("{}", warning),
			loc: None,
			module_name: None,
			module_path: None,
		}
	}

	pub fn with_pos(self, loc: (usize, usize)) -> Diagnostic {
		Diagnostic {
			loc: Some(loc),
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

pub fn print_diagnostics(diagnostics: Vec<Diagnostic>) {
	let mut first = true;
	let mut module_bytes = HashMap::<PathBuf, Vec<u8>>::new();

	for diagnostic in diagnostics {
		if !first {
			eprintln!("")
		}

		let is_error = diagnostic.is_error();

		eprintln!(
			"{} {}",
			if is_error {
				colors::bold_red("Error:")
			} else {
				colors::bold_yellow("Warning:")
			},
			diagnostic.message
		);

		if diagnostic.module_path.is_none() {
			continue;
		}

		let mut module_path = diagnostic.module_path.unwrap();
		let cwd = std::env::current_dir().unwrap_or(PathBuf::from(""));
		if module_path.starts_with(cwd) {
			module_path = module_path
				.strip_prefix(std::env::current_dir().unwrap())
				.unwrap()
				.to_path_buf();
		}

		if let Some((start, end)) = diagnostic.loc {
			let mut col_index = 0;

			let bytes = match module_bytes.get(&module_path) {
				Some(bytes) => bytes,
				None => match fs::read(&module_path) {
					Ok(bytes) => {
						module_bytes.insert(module_path.clone(), bytes);
						module_bytes.get(&module_path).unwrap()
					}
					_ => unreachable!(),
				},
			};

			let mut frame_start = start;
			let mut frame_end = end;

			while frame_start > 0 {
				if let Some(b'\n') = bytes.get(frame_start - 1) {
					break;
				}

				col_index += 1;
				frame_start -= 1
			}

			while let Some(byte) = bytes.get(frame_end) {
				match byte {
					b'\n' => break,
					_ => frame_end += 1,
				}
			}

			let frame = String::from_utf8(bytes[frame_start..frame_end].to_vec())
				.unwrap()
				.replace("\t", " ")
				.replace("\n", " ");

			let mut line = 1;

			frame_start = start;
			while frame_start > 0 {
				if let Some(b'\n') = bytes.get(frame_start - 1) {
					line += 1;
				}

				frame_start -= 1;
			}

			eprintln!(
				"\n{} {} {}",
				if is_error {
					colors::bold_red(">")
				} else {
					colors::bold_yellow(">")
				},
				colors::bold_dim(format!("{}|", line).as_str()),
				frame
			);

			let prefix_width = 4 + line.to_string().len();
			let up_arrows = "^".repeat(end - start).to_string();

			eprintln!(
				"{}{}",
				" ".repeat(prefix_width + col_index),
				if is_error {
					colors::bold_red(&up_arrows)
				} else {
					colors::bold_yellow(&up_arrows)
				}
			);

			eprintln!(
				"{}",
				colors::dim(
					format!(
						"{}:{}:{}",
						module_path.to_str().unwrap(),
						line,
						col_index + 1
					)
					.as_str()
				)
			);
		}

		first = false;
	}
}
