use crate::analyzer::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::module::*;
use crate::stdlib::lookup_stdlib_source;
use crate::*;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};

// Native modules every user module sees without an explicit `use`. The
// local name is what user code references it as. Codegen reads the same
// list to mirror the analyzer's view of what's in scope.
//
// `option` and `result` share their local name with the prelude enums of
// the same name (intentional — `option.then` resolves to the module's
// `then`, `option.some` falls through to the enum's `some`). The
// FieldAccess dispatch in the analyzer handles the overlap.
pub const AUTO_IMPORTS: &[(&str, &str)] = &[
	("core.ref", "ref"),
	("core.option", "option"),
	("core.result", "result"),
	// NB: `core.task` is deliberately *not* auto-imported. The async syntax
	// (`try`/`??` over a task, `scope`, `defer`, duration literals) needs no
	// import — it's type-driven and lowers to fully-qualified globals — but
	// every *named* task function (`task.return`, `task.sleep`, `task.both`,
	// the kernel behind `s.spawn`/`s.next`, …) lives behind `use core.task`.
	// Since you can't build a task without `task.return`/`task.fail`, async
	// code imports `core.task` anyway; keeping it explicit avoids pulling the
	// whole combinator surface into every module's namespace.
];

// PLUMA_TIMING=1 prints per-module parse/analyze wall-clock to stderr.
fn timing_log(module: &str, phase: &str, dur: std::time::Duration) {
	if std::env::var("PLUMA_TIMING").is_ok() {
		eprintln!(
			"  [{:>7}] {:>8.2} ms  {}",
			phase,
			dur.as_secs_f64() * 1000.0,
			module
		);
	}
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct Compiler {
	pub root_dir: PathBuf,
	// Modules to load as roots of the import graph. `pluma run` has exactly
	// one (the file the user asked to execute); `pluma test` has one per
	// discovered `*.test.pa` file. Codegen, LSP, etc. that previously read a
	// single entry module name read `entry_modules[0]` instead.
	pub entry_modules: Vec<String>,
	pub modules: HashMap<String, Module>,
	diagnostics: Vec<Diagnostic>,
	// Per fully-qualified module name, the top-level defs (values, aliases,
	// enums) other modules see when they `use` it.
	exports_cache: HashMap<String, ModuleExports>,
	// Pre-registered native modules (stdlib). Resolved without parsing any
	// `.pa` file — the compiler hands the analyzer their exports directly.
	pub native_modules: HashMap<String, ModuleExports>,
	// The target platform whose host-capability profile gates module
	// availability (a `use core.io` on the browser target is an error).
	// Defaults to `Native` (the VM/dev profile — provides every capability,
	// so nothing is gated), so existing flows are unchanged.
	pub platform: Platform,
}

impl Compiler {
	pub fn from_entry_path(entry_path: String) -> Result<Self, Vec<Diagnostic>> {
		let (root_dir, entry_module_name) = resolve_entry(entry_path)?;

		Ok(Compiler {
			root_dir,
			entry_modules: vec![entry_module_name],
			modules: HashMap::new(),
			diagnostics: Vec::new(),
			exports_cache: HashMap::new(),
			native_modules: HashMap::new(),
			platform: Platform::default(),
		})
	}

	// Select the target platform whose capability profile gates module
	// availability. Builder form so the ~14 existing constructor call sites
	// (cli, lsp, tests, bench) keep their default `Native` profile untouched;
	// only a platform-specific build (e.g. the wasm-server test harness) opts in.
	pub fn with_platform(mut self, platform: Platform) -> Self {
		self.platform = platform;
		self
	}

	// Construct a compiler rooted at `root_dir` with no entry modules. The
	// caller registers each entry module via `add_entry_module` — used by
	// `pluma test`, which discovers `*.test.pa` files itself and feeds them
	// in as roots rather than relying on a single `main.pa`.
	pub fn for_root_dir(root_dir: PathBuf) -> Self {
		Compiler {
			root_dir,
			entry_modules: Vec::new(),
			modules: HashMap::new(),
			diagnostics: Vec::new(),
			exports_cache: HashMap::new(),
			native_modules: HashMap::new(),
			platform: Platform::default(),
		}
	}

	pub fn add_entry_module(&mut self, module_name: String) {
		self.entry_modules.push(module_name);
	}

	// Register a stdlib module (e.g. `core.regex`) so its exports are visible
	// to any user module that does `use <name>`. Must be called before
	// `check()`. The runtime values come from the VM side
	// (`vm::stdlib::register_compiler`).
	pub fn register_native_module(&mut self, name: String, exports: ModuleExports) {
		self.native_modules.insert(name, exports);
	}

	// Pre-parse a module from in-memory bytes and insert it into the
	// module cache. A later `check()` call sees this module as already
	// parsed and skips the disk read for it. Lets editor/LSP integrations
	// analyze unsaved changes without writing to disk.
	pub fn set_module_source(&mut self, module_name: String, source: Vec<u8>) {
		let path = to_module_path(&self.root_dir, &module_name);
		let mut module = Module::new(module_name.clone(), path);
		module.parse_from_bytes(source, &mut self.diagnostics);
		self.modules.insert(module_name, module);
	}

	pub fn tokenize(&mut self) -> Result<Vec<Token>, Vec<Diagnostic>> {
		let entry = self
			.entry_modules
			.first()
			.cloned()
			.expect("tokenize() called with no entry modules");
		let mut entry_module = Module::new(entry.clone(), to_module_path(&self.root_dir, &entry));

		let tokens = entry_module.tokenize(&mut self.diagnostics);

		Ok(tokens)
	}

	pub fn check(&mut self) -> Result<(), Vec<Diagnostic>> {
		// Load + analyze the baked-in `__prelude__` module before anything
		// else. Its exported instances are implicitly visible to every
		// user module's analyzer.
		self.load_prelude();
		let mut visiting = HashSet::new();
		for entry in self.entry_modules.clone() {
			self.load_module(&entry, &mut visiting);
		}

		if !self.diagnostics.is_empty() {
			Err(self.diagnostics.to_vec())
		} else {
			Ok(())
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
		let _t = std::time::Instant::now();
		analyzer.analyze(module);
		timing_log(NAME, "analyze", _t.elapsed());
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

		// Baked-in stdlib `.pa` source: parse + analyze like a user module
		// but pull bytes from the in-memory registry. Takes precedence over
		// any same-named pre-registered native module — once a stdlib
		// module is expressed in Pluma, the Rust side stops shipping its
		// type table.
		let stdlib_source = lookup_stdlib_source(module_name);

		if stdlib_source.is_none() {
			// Native stdlib modules: pull pre-registered exports into the cache
			// and skip parse/analyze entirely.
			if let Some(exports) = self.native_modules.get(module_name).cloned() {
				self.exports_cache.insert(module_name.to_string(), exports);
				return;
			}
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
			let _t = std::time::Instant::now();
			if let Some(source) = stdlib_source {
				let mut module = Module::new(
					module_name.to_string(),
					PathBuf::from(format!("<stdlib:{}>", module_name)),
				);
				module.parse_from_bytes(source.as_bytes().to_vec(), &mut self.diagnostics);
				self.modules.insert(module_name.to_string(), module);
			} else {
				let path = to_module_path(&self.root_dir, module_name);
				let mut module = Module::new(module_name.to_string(), path);
				module.parse(&mut self.diagnostics);
				self.modules.insert(module_name.to_string(), module);
			}
			timing_log(module_name, "parse", _t.elapsed());
		}

		// Collect (fully-qualified-name, local-namespace-name, alias-range,
		// use-statement-range) for each import. Local namespace name is the alias
		// if present, otherwise the last dotted segment — so `use sub.utils` binds
		// `utils` and `use sub.utils as u` binds `u`. The use-statement range spans
		// the whole `use …` line (a better caret target for platform gating than
		// the alias).
		let imports: Vec<(String, String, Range, Range)> = self
			.modules
			.get(module_name)
			.and_then(|m| m.ast.as_ref())
			.map(|ast| {
				ast
					.uses
					.iter()
					.map(|u| {
						let local = u.local_name();
						(u.module_name(), local.name.clone(), local.range, u.range)
					})
					.collect()
			})
			.unwrap_or_default();

		// Check for two imports binding the same local name. The second one wins
		// silently otherwise.
		let mut seen: HashMap<String, Range> = HashMap::new();
		for (_, local_name, range, _) in &imports {
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

		// Test modules (name ends in `.test`) are only importable by other
		// test modules. Production code shouldn't depend on test-only files.
		// The project marker (`pluma.pa`) is config, not a library — it's
		// never importable by anything else.
		let importer_is_test = module_name.ends_with(".test");
		let importer_path = self.modules.get(module_name).map(|m| m.module_path.clone());
		let mut rejected_imports: HashSet<String> = HashSet::new();
		for (full_name, _, range, use_range) in &imports {
			// The capabilities `full_name` requires that the active platform
			// doesn't provide. Empty on the default `Native` profile (it provides
			// everything) and for any ungated module — so this gate is inert for
			// existing flows. Reported against the whole `use …` statement.
			let missing_caps = self.platform.missing_capabilities(full_name);
			let rejection: Option<(String, Range)> = if full_name.ends_with(".test") && !importer_is_test
			{
				Some((
					format!(
						"Cannot import test module `{}` from a non-test module. \
						Only `.test` modules may `use` other `.test` modules.",
						full_name
					),
					*range,
				))
			} else if full_name == PROJECT_MARKER_MODULE && module_name != PROJECT_MARKER_MODULE {
				Some((
					format!(
						"Cannot `use {}` — the project marker file is config, not \
						a library. Project metadata is one-directional: the CLI reads \
						it, runtime code never depends on it.",
						full_name
					),
					*range,
				))
			} else if !missing_caps.is_empty() {
				Some((
					format!(
						"`{}` is not available on the {} target — it needs host \
						capabilities {:?} this platform does not provide.",
						full_name,
						self.platform.label(),
						missing_caps
					),
					*use_range,
				))
			} else {
				None
			};
			if let Some((message, at)) = rejection {
				let mut diag = Diagnostic::error(message).with_range(at);
				if let Some(path) = importer_path.clone() {
					diag = diag.with_module(module_name.to_string(), path);
				}
				self.diagnostics.push(diag);
				rejected_imports.insert(full_name.clone());
			}
		}

		// Recursively load each dependency that wasn't rejected above.
		// Loading rejected imports anyway would produce confusing cascade
		// errors (e.g. "value X not found in import") in addition to the
		// real cause.
		for (full_name, _, _, _) in &imports {
			if rejected_imports.contains(full_name) {
				continue;
			}
			self.load_module(full_name, visiting);
		}

		// Build the imports map for the analyzer (local name -> exports table),
		// plus a parallel local-name -> fully-qualified-module-name map so
		// qualified enum type names can be reconstructed at use sites.
		let mut imports_map: HashMap<String, ModuleExports> = HashMap::new();
		let mut import_qualified: HashMap<String, String> = HashMap::new();
		for (full_name, local_name, _, _) in imports {
			if rejected_imports.contains(&full_name) {
				continue;
			}
			if let Some(exports) = self.exports_cache.get(&full_name) {
				imports_map.insert(local_name.clone(), exports.clone());
				import_qualified.insert(local_name, full_name);
			}
		}

		// Auto-imported modules: bound under a bare name in every user
		// module without an explicit `use`. Currently `core.ref` →
		// `ref`, `core.option` → `option`, `core.result` → `result`.
		// User code can shadow by binding the local name to something
		// else via `use`. Exports come from either a baked `.pa` source
		// (loaded via `load_module` into `exports_cache`) or from a
		// pre-registered native module. Auto-imports don't apply when
		// loading an auto-imported module itself — they'd otherwise
		// form a cycle among themselves (loading `core.option` would
		// recurse into loading `core.ref` etc. while `core.option` is
		// still on the visiting stack).
		let is_auto_imported_module = AUTO_IMPORTS.iter().any(|(n, _)| *n == module_name);
		if !is_auto_imported_module {
			for (full_name, local_name) in AUTO_IMPORTS {
				if imports_map.contains_key(*local_name) {
					continue;
				}
				self.load_module(full_name, visiting);
				if let Some(exports) = self.exports_cache.get(*full_name).cloned() {
					imports_map.insert(local_name.to_string(), exports);
					import_qualified.insert(local_name.to_string(), full_name.to_string());
				}
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
		let _t = std::time::Instant::now();
		analyzer.analyze(module);
		timing_log(module_name, "analyze", _t.elapsed());

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

// Name of the file marking a Pluma package root.
pub const PROJECT_MARKER_FILE: &str = "pluma.pa";

// Module name of the project marker, used to identify it in import-rejection
// checks and special analyzer rules.
pub const PROJECT_MARKER_MODULE: &str = "pluma";

// Walk upward from `start` looking for a directory containing
// `pluma.pa`. Returns the first directory that has one. None means no
// marker was found anywhere on the path to the filesystem root.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
	let mut dir = if start.is_dir() {
		start.to_path_buf()
	} else {
		start.parent()?.to_path_buf()
	};
	loop {
		if dir.join(PROJECT_MARKER_FILE).is_file() {
			return Some(dir);
		}
		if !dir.pop() {
			return None;
		}
	}
}

pub fn to_module_path(root_dir: &Path, module_name: &str) -> PathBuf {
	let mut path = root_dir.to_path_buf();
	// Test modules live in `<segments>.test.pa` files. Their module name keeps
	// the `.test` suffix (e.g. `foo.bar.test`), so we peel that off before
	// turning intermediate dots into path separators.
	if let Some(stem) = module_name.strip_suffix(".test") {
		for segment in stem.split('.') {
			path.push(segment);
		}
		path.set_extension(format!("test.{}", FILE_EXTENSION));
	} else {
		for segment in module_name.split('.') {
			path.push(segment);
		}
		path.set_extension(FILE_EXTENSION);
	}
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
		Ok(abs_path) => {
			let entry_dir = abs_path.parent().unwrap().to_path_buf();
			// Walk up from the entry file's directory looking for `pluma.pa`.
			// If found, the package root anchors module-name resolution and the
			// entry becomes a dotted path relative to it (e.g. `auth.login`).
			// Otherwise fall back to the legacy rule — entry file's directory
			// is the root, entry module is the file stem.
			let (root_dir, module_name) = match find_project_root(&entry_dir) {
				Some(root) => {
					let rel = abs_path.strip_prefix(&root).unwrap_or(&abs_path);
					let stem_path = rel.with_extension("");
					let module_name = stem_path
						.to_string_lossy()
						.replace(std::path::MAIN_SEPARATOR, ".");
					(root, module_name)
				}
				None => (
					entry_dir,
					abs_path.file_stem().unwrap().to_str().unwrap().to_owned(),
				),
			};
			Ok((root_dir, module_name))
		}

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

#[cfg(test)]
mod platform_gating_tests {
	use super::*;

	// Compile a synthetic `main` module under `platform`, returning the
	// diagnostics (empty on success). The module source is fed in-memory, so no
	// disk access is needed; gated stdlib modules resolve from the baked sources.
	fn check_with(platform: Platform, source: &str) -> Vec<Diagnostic> {
		let mut compiler = Compiler::for_root_dir(std::env::temp_dir()).with_platform(platform);
		compiler.set_module_source("main".to_string(), source.as_bytes().to_vec());
		compiler.add_entry_module("main".to_string());
		match compiler.check() {
			Ok(()) => Vec::new(),
			Err(diags) => diags,
		}
	}

	#[test]
	fn core_io_allowed_on_native_and_server() {
		let src = "use core.io\n\ndef main = fun { io.print \"hi\" }\n";
		assert!(check_with(Platform::Native, src).is_empty());
		assert!(check_with(Platform::Server, src).is_empty());
	}

	#[test]
	fn core_io_rejected_on_browser() {
		let src = "use core.io\n\ndef main = fun { io.print \"hi\" }\n";
		let diags = check_with(Platform::Browser, src);
		assert!(
			diags
				.iter()
				.any(|d| d.message.contains("core.io") && d.message.contains("browser")),
			"expected a browser-target rejection for core.io, got: {:?}",
			diags.iter().map(|d| &d.message).collect::<Vec<_>>()
		);
	}

	#[test]
	fn ungated_module_available_everywhere() {
		let src = "use core.list\n\ndef main = fun { list.length [1] }\n";
		for p in [Platform::Native, Platform::Server, Platform::Browser] {
			assert!(
				check_with(p, src).is_empty(),
				"core.list rejected on {:?}",
				p
			);
		}
	}
}
