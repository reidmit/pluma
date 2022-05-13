use crate::diagnostic::*;
use crate::errors::*;
use crate::module::*;
use crate::*;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Compiler {
	pub root_dir: PathBuf,
	pub entry_module_name: String,
	pub modules: HashMap<String, Module>,
	diagnostics: Vec<Diagnostic>,
}

impl Compiler {
	pub fn from_entry_path(entry_path: String) -> Result<Self, Vec<Diagnostic>> {
		let (root_dir, entry_module_name) = resolve_entry(entry_path)?;

		Ok(Compiler {
			root_dir,
			entry_module_name,
			modules: HashMap::new(),
			diagnostics: Vec::new(),
		})
	}

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

	pub fn check(&mut self) -> Result<(), Vec<Diagnostic>> {
		self.parse()?;

		if !self.diagnostics.is_empty() {
			return Err(self.diagnostics.to_vec());
		}

		Ok(())
	}

	fn parse_module(&mut self, module_name: String, module_path: PathBuf) {
		if self.modules.contains_key(&module_name) {
			return;
		};

		let mut new_module = Module::new(module_name.clone(), module_path.to_owned());

		let result = new_module.parse();

		println!("AST: {:#?}", new_module.ast);

		self.modules.insert(module_name.clone(), new_module);

		if !result.is_ok() {
			self.diagnostics.append(&mut result.unwrap_err());
			return;
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
	root_dir.join(module_name).with_extension(FILE_EXTENSION)
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
