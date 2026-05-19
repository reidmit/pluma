use crate::analyzer::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::module::*;
use crate::*;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};

// Native modules every user module sees without an explicit `use`. The
// local name is what user code references it as. Codegen reads the same
// list to mirror the analyzer's view of what's in scope.
pub const AUTO_IMPORTS: &[(&str, &str)] = &[("core.ref", "ref")];

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Compiler {
	pub root_dir: PathBuf,
	pub entry_module_name: String,
	pub modules: HashMap<String, Module>,
	diagnostics: Vec<Diagnostic>,
	// Per fully-qualified module name, the top-level defs (values, aliases,
	// enums) other modules see when they `use` it.
	exports_cache: HashMap<String, ModuleExports>,
	// Pre-registered native modules (stdlib). Resolved without parsing any
	// `.pa` file — the compiler hands the analyzer their exports directly.
	pub native_modules: HashMap<String, ModuleExports>,
}

impl Compiler {
	pub fn from_entry_path(entry_path: String) -> Result<Self, Vec<Diagnostic>> {
		let (root_dir, entry_module_name) = resolve_entry(entry_path)?;

		Ok(Compiler {
			root_dir,
			entry_module_name,
			modules: HashMap::new(),
			diagnostics: Vec::new(),
			exports_cache: HashMap::new(),
			native_modules: HashMap::new(),
		})
	}

	// Register a stdlib module (e.g. `core.regex`) so its exports are visible
	// to any user module that does `use <name>`. Must be called before
	// `check()`. The runtime values come from the VM side
	// (`vm::stdlib::register_compiler`).
	pub fn register_native_module(&mut self, name: String, exports: ModuleExports) {
		self.native_modules.insert(name, exports);
	}

	pub fn tokenize(&mut self) -> Result<Vec<Token>, Vec<Diagnostic>> {
		let mut entry_module = Module::new(
			self.entry_module_name.clone(),
			to_module_path(&self.root_dir, &self.entry_module_name),
		);

		let tokens = entry_module.tokenize(&mut self.diagnostics);

		Ok(tokens)
	}

	pub fn check(&mut self) -> Result<&Module, Vec<Diagnostic>> {
		// Load + analyze the baked-in `__prelude__` module before anything
		// else. Its exported instances are implicitly visible to every
		// user module's analyzer.
		self.load_prelude();
		let entry = self.entry_module_name.clone();
		let mut visiting = HashSet::new();
		self.load_module(&entry, &mut visiting);

		if !self.diagnostics.is_empty() {
			Err(self.diagnostics.to_vec())
		} else {
			Ok(self.modules.get(&entry).unwrap())
		}
	}

	// Parse + analyze the synthetic prelude module. The source is baked
	// into the compiler binary so the language doesn't depend on a
	// stdlib install directory.
	fn load_prelude(&mut self) {
		const PRELUDE_SOURCE: &str = include_str!("prelude.pa");
		const NAME: &str = "__prelude__";
		let mut module = Module::new(NAME.to_string(), PathBuf::from("<prelude>"));
		module.parse_from_bytes(PRELUDE_SOURCE.as_bytes().to_vec(), &mut self.diagnostics);
		self.modules.insert(NAME.to_string(), module);
		// Analyze in isolation — prelude has no imports.
		let module = self.modules.get_mut(NAME).unwrap();
		let mut analyzer = Analyzer::new(&mut self.diagnostics);
		analyzer.set_imports(HashMap::new(), HashMap::new());
		analyzer.analyze(module);
		if let Some(exports) = module.exports.clone() {
			self.exports_cache.insert(NAME.to_string(), exports);
		}
	}

	// DFS-loads `module_name` and its imports, then analyzes it. Each module
	// is analyzed once, after its dependencies. Detects import cycles via
	// `visiting`.
	fn load_module(&mut self, module_name: &str, visiting: &mut HashSet<String>) {
		if self.exports_cache.contains_key(module_name) {
			return;
		}

		// Native stdlib modules: pull pre-registered exports into the cache
		// and skip parse/analyze entirely.
		if let Some(exports) = self.native_modules.get(module_name).cloned() {
			self.exports_cache.insert(module_name.to_string(), exports);
			return;
		}

		if !visiting.insert(module_name.to_string()) {
			self.diagnostics.push(Diagnostic::error(format!(
				"Cyclic import detected involving module `{}`.",
				module_name
			)));
			return;
		}

		// Parse if not already.
		if !self.modules.contains_key(module_name) {
			let path = to_module_path(&self.root_dir, module_name);
			let mut module = Module::new(module_name.to_string(), path);
			module.parse(&mut self.diagnostics);
			self.modules.insert(module_name.to_string(), module);
		}

		// Collect (fully-qualified-name, local-namespace-name, alias-range) for
		// each import. Local namespace name is the alias if present, otherwise
		// the last dotted segment — so `use sub.utils` binds `utils` and
		// `use sub.utils as u` binds `u`.
		let imports: Vec<(String, String, Range)> = self
			.modules
			.get(module_name)
			.and_then(|m| m.ast.as_ref())
			.map(|ast| {
				ast
					.uses
					.iter()
					.map(|u| {
						let local = u.local_name();
						(u.module_name(), local.name.clone(), local.range)
					})
					.collect()
			})
			.unwrap_or_default();

		// Check for two imports binding the same local name. The second one wins
		// silently otherwise.
		let mut seen: HashMap<String, Range> = HashMap::new();
		for (_, local_name, range) in &imports {
			if let Some(_prev) = seen.insert(local_name.clone(), *range) {
				self.diagnostics.push(
					Diagnostic::error(format!(
						"Duplicate import name `{}`. Add an `as` alias to disambiguate.",
						local_name
					))
					.with_range(*range),
				);
			}
		}

		// Recursively load each dependency.
		for (full_name, _, _) in &imports {
			self.load_module(full_name, visiting);
		}

		// Build the imports map for the analyzer (local name -> exports table),
		// plus a parallel local-name -> fully-qualified-module-name map so
		// qualified enum type names can be reconstructed at use sites.
		let mut imports_map: HashMap<String, ModuleExports> = HashMap::new();
		let mut import_qualified: HashMap<String, String> = HashMap::new();
		for (full_name, local_name, _) in imports {
			if let Some(exports) = self.exports_cache.get(&full_name) {
				imports_map.insert(local_name.clone(), exports.clone());
				import_qualified.insert(local_name, full_name);
			}
		}

		// Auto-imported modules: bound under a bare name in every user
		// module without an explicit `use`. Currently just `core.ref` →
		// `ref` so mutable cells are reachable without ceremony. User
		// code can shadow by binding `ref` to something else via `use`.
		for (full_name, local_name) in AUTO_IMPORTS {
			if imports_map.contains_key(*local_name) {
				continue;
			}
			if let Some(exports) = self.native_modules.get(*full_name).cloned() {
				self.exports_cache
					.entry(full_name.to_string())
					.or_insert_with(|| exports.clone());
				imports_map.insert(local_name.to_string(), exports);
				import_qualified.insert(local_name.to_string(), full_name.to_string());
			}
		}

		// Analyze this module. The prelude's exports (enums, variant
		// constructors, instances) are implicitly available — pass them
		// in alongside explicit imports so name resolution + discharge
		// can use them.
		let prelude_exports = self.exports_cache.get("__prelude__").cloned();
		let module = self.modules.get_mut(module_name).unwrap();
		let mut analyzer = Analyzer::new(&mut self.diagnostics);
		analyzer.set_imports(imports_map, import_qualified);
		if let Some(exports) = prelude_exports {
			analyzer.add_imported_instances(&exports.instances);
			analyzer.set_prelude_exports(exports);
		}
		analyzer.analyze(module);

		// Cache its exports for any later importer.
		if let Some(exports) = module.exports.clone() {
			self.exports_cache.insert(module_name.to_string(), exports);
		}

		visiting.remove(module_name);
	}
}

fn resolve_entry(entry_path: String) -> Result<(PathBuf, String), Vec<Diagnostic>> {
	match get_root_dir_and_module_name(entry_path) {
		Ok(result) => Ok(result),
		Err(usage_error) => Err(vec![Diagnostic::error(usage_error)]),
	}
}

fn to_module_path(root_dir: &Path, module_name: &str) -> PathBuf {
	let mut path = root_dir.to_path_buf();
	for segment in module_name.split('.') {
		path.push(segment);
	}
	path.set_extension(FILE_EXTENSION);
	path
}

fn get_root_dir_and_module_name(entry_path: String) -> Result<(PathBuf, String), UsageError> {
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
