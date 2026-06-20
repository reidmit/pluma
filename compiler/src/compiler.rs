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
	("std/ref", "ref"),
	("std/option", "option"),
	("std/result", "result"),
	// NB: `std/task` is deliberately *not* auto-imported. The async syntax
	// (`try`/`??` over a task, `scope`, `defer`, duration literals) needs no
	// import — it's type-driven and lowers to fully-qualified globals — but
	// every *named* task function (`task.return`, `task.sleep`, `task.both`,
	// the kernel behind `s.spawn`/`s.next`, …) lives behind `use std/task`.
	// Since you can't build a task without `task.return`/`task.fail`, async
	// code imports `std/task` anyway; keeping it explicit avoids pulling the
	// whole combinator surface into every module's namespace.
];

// One module's analyzed exports plus the hash of the source they came from.
// The hash gates reuse: a cached entry is valid only while the module's
// source is byte-identical to what produced it. Only diagnostic-free
// analyses are cached (see `load_module`), so reusing an entry can never
// silently drop a diagnostic.
#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct ModuleCacheEntry {
	pub source_hash: u64,
	pub exports: ModuleExports,
}

// A persistent, content-addressed export cache keyed by fully-qualified
// module name. A long-lived caller (the LSP) owns one across `Compiler`
// instances, swapping it in before `check()` and back out after, so an edit
// to one file doesn't force re-analysis of the unchanged modules it imports.
pub type ModuleCache = HashMap<String, ModuleCacheEntry>;

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
	// Iterate this via `modules_sorted()`, not `.iter()` — hash order is
	// per-process-random and any id assigned in iteration order must be stable.
	pub modules: HashMap<String, Module>,
	diagnostics: Vec<Diagnostic>,
	// Per fully-qualified module name, the top-level defs (values, aliases,
	// enums) other modules see when they `use` it.
	exports_cache: HashMap<String, ModuleExports>,
	// Pre-registered native modules (stdlib). Resolved without parsing any
	// `.pa` file — the compiler hands the analyzer their exports directly.
	pub native_modules: HashMap<String, ModuleExports>,
	// The deploy target whose tier gates module availability (a `use std/web/dom`
	// on the `sys` target is an error). `None` is the ungated frontend/analysis
	// mode — nothing is gated — so existing flows are unchanged.
	pub target: Option<Target>,
	// `pluma dev` hot-reload mode: the analyzer redirects `app.sandbox`/`app.element`
	// to their model-persisting `-hmr` variants. Off everywhere else.
	pub hmr: bool,
	// FULLSTACK dual build (`main.pa` + `client.pa` in one directory). The two
	// entries compile from one `check()`, then emit twice; gating runs per artifact
	// (`entry_modules[0]`=server→`Sys`, `[1]`=client→`Web`) via `gate_fullstack`, and
	// the generated `rpc-client` targets the web transport (`fetch.post`).
	pub fullstack: bool,
	// The server origin the synthesized RPC client stubs POST to, baked at build
	// time into `std/rpc.server-origin` (`pluma build/dev --server-url`). `None`
	// keeps the built-in fallback. Empty string = same-origin (`/_rpc/...`).
	pub rpc_base_url: Option<String>,
	// Every `remote def` discovered during analysis, with its resolved wire
	// shapes + per-route fingerprint. The lowerer reads this to synthesize the
	// client stub bodies and the `rpc-dispatch` routing table directly as IR.
	pub rpc_endpoints: Vec<crate::rpc::RpcEndpointMeta>,
	// Optional cross-compile export cache for incremental re-analysis. When
	// set (the LSP swaps one in per keystroke), a module whose source is
	// unchanged and whose dependencies were all reused skips re-analysis and
	// reuses its cached exports. `None` for one-shot compiles (CLI), which
	// always analyze everything.
	incremental: Option<ModuleCache>,
	// Modules (re)analyzed during the current `check()` pass — its source
	// changed, or a dependency was reanalyzed. A module isn't reused if any of
	// its imports is in here, so a signature change propagates to dependents.
	reanalyzed: HashSet<String>,
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
			target: None,
			hmr: false,
			fullstack: false,
			rpc_base_url: None,
			rpc_endpoints: Vec::new(),
			incremental: None,
			reanalyzed: HashSet::new(),
		})
	}

	// Select the deploy target whose tier gates module availability. Builder
	// form so the ~14 existing constructor call sites (cli, lsp, tests) keep
	// their default ungated (`None`) mode untouched; only `pluma build` opts
	// into gating (`Web` with `--web`, `Sys` otherwise).
	pub fn with_target(mut self, target: Option<Target>) -> Self {
		self.target = target;
		self
	}

	// Enable `pluma dev` hot-reload redirection. Builder form like `with_target`,
	// so existing call sites default to `false` (no redirection).
	pub fn with_hmr(mut self, hmr: bool) -> Self {
		self.hmr = hmr;
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
			target: None,
			hmr: false,
			fullstack: false,
			rpc_base_url: None,
			rpc_endpoints: Vec::new(),
			incremental: None,
			reanalyzed: HashSet::new(),
		}
	}

	// Analyze the entire baked-in stdlib — the prelude plus every `std/*`
	// module — once, and return the resulting export tables keyed by
	// fully-qualified module name. The stdlib source is immutable for the life
	// of the binary, so a long-lived consumer (the LSP, which re-analyzes the
	// user's file on every keystroke) computes this table once and seeds each
	// fresh per-edit `Compiler` with it via `seed_exports`. A seeded module
	// short-circuits in `load_module` before parse+analyze, so the stdlib —
	// the dominant fixed cost of a single-file analysis — is paid for once per
	// process instead of once per keystroke. Standalone: builds a throwaway
	// compiler carrying no entry modules or user state. No tier gating runs
	// (`target` is `None`), so both the `sys` and `web` tiers are analyzed.
	pub fn stdlib_export_table() -> HashMap<String, ModuleExports> {
		let mut compiler = Compiler::for_root_dir(PathBuf::from("."));
		compiler.load_prelude();
		let mut visiting = HashSet::new();
		for &(name, _) in crate::stdlib_sources() {
			compiler.load_module(name, &mut visiting);
		}
		compiler.exports_cache
	}

	// Enable incremental re-analysis backed by `cache` (see `ModuleCache`).
	// `check()` reuses any cached module whose source is unchanged and whose
	// dependencies were all reused; reclaim the updated cache afterward with
	// `take_incremental_cache`. Must be called before `check()`.
	pub fn enable_incremental(&mut self, cache: ModuleCache) {
		self.incremental = Some(cache);
	}

	// Reclaim the incremental cache after `check()`, with entries for every
	// module analyzed cleanly this pass refreshed. Returns an empty map if
	// incremental mode wasn't enabled.
	pub fn take_incremental_cache(&mut self) -> ModuleCache {
		self.incremental.take().unwrap_or_default()
	}

	// The modules (re)analyzed during the last `check()` — those whose source
	// changed or whose dependencies did. A module absent here was reused from
	// the incremental cache. Exposed for tests and tooling that want to see
	// what an edit actually re-touched.
	pub fn reanalyzed_modules(&self) -> &HashSet<String> {
		&self.reanalyzed
	}

	// Pre-populate the export cache with a precomputed table (see
	// `stdlib_export_table`). Existing entries win, so a user module that
	// happens to shadow a seeded name still takes precedence. Must be called
	// before `check()`.
	pub fn seed_exports(&mut self, seed: &HashMap<String, ModuleExports>) {
		for (name, exports) in seed {
			self
				.exports_cache
				.entry(name.clone())
				.or_insert_with(|| exports.clone());
		}
	}

	// Mark this as a FULLSTACK dual build (`main.pa` + `client.pa`). The driver
	// sets `entry_modules = [server, client]`; this flag tells RPC codegen the
	// client is web (`fetch.post` transport) and enables `gate_fullstack`.
	pub fn with_fullstack(mut self, fullstack: bool) -> Self {
		self.fullstack = fullstack;
		self
	}

	// Set the server origin the generated `rpc-client` stubs default to (`pluma
	// build/dev --server-url`). Must be called before `check()` (RPC codegen reads
	// it). An empty string bakes a same-origin base (`/_rpc/...`).
	pub fn with_rpc_base_url(mut self, url: String) -> Self {
		self.rpc_base_url = Some(url);
		self
	}

	pub fn add_entry_module(&mut self, module_name: String) {
		self.entry_modules.push(module_name);
	}

	// Whether `entry_path` names a FULLSTACK project directory — one holding BOTH
	// `main.pa` and `client.pa`. The driver dispatches on this to build two
	// artifacts from one source (`from_fullstack_dir`).
	pub fn is_fullstack_dir(entry_path: &str) -> bool {
		let dir = Path::new(entry_path);
		dir.is_dir() && dir.join("main.pa").is_file() && dir.join("client.pa").is_file()
	}

	// Construct a FULLSTACK compiler from a directory holding `main.pa` (the server)
	// + `client.pa`: `entry_modules = [server, client]`, `fullstack = true`. Module
	// names are resolved by the normal rule (honoring a `pluma.pa` package root above,
	// if any), so both halves and their shared modules share one root.
	pub fn from_fullstack_dir(entry_path: String) -> Result<Self, Vec<Diagnostic>> {
		let mut compiler = Self::from_entry_path(format!("{entry_path}/main"))?;
		let (_, client) = resolve_entry(format!("{entry_path}/client"))?;
		compiler.entry_modules.push(client);
		compiler.fullstack = true;
		Ok(compiler)
	}

	// Register a Rust-defined native module's exports so any user module that
	// does `use <name>` type-checks against them. Must be called before
	// `check()`. Currently unused — every stdlib module is a `.pa` source — but
	// kept for any future module whose signature the `.pa` surface can't express.
	pub fn register_native_module(&mut self, name: String, exports: ModuleExports) {
		self.native_modules.insert(name, exports);
	}

	/// Every loaded module in a canonical (name-sorted) order. **Always iterate
	/// modules through this, never `self.modules.iter()` directly.** `modules` is
	/// a `HashMap`, whose iteration order is seeded per process; a pass that
	/// assigns sequential ids (FuncId/GlobalId) or emits in module order would
	/// then produce different — and occasionally miscompiled — wasm each run.
	/// Routing all iteration through one sorted accessor makes the build a pure
	/// function of the source and keeps a future iteration site from silently
	/// reintroducing that non-determinism. (Point `get`/`insert` are unaffected.)
	pub fn modules_sorted(&self) -> Vec<(&String, &Module)> {
		let mut modules: Vec<(&String, &Module)> = self.modules.iter().collect();
		modules.sort_by(|a, b| a.0.cmp(b.0));
		modules
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

		// If the build has `remote def`s and will emit a client artifact (a `--web`
		// build, or the client half of a fullstack build), force-load the web
		// transport so the lowerer can reference its `install` (no user code `use`s
		// it). It's outside every root's use-graph, so tier gating never traverses
		// it — its `std/web/*` imports don't bar the sys server. DCE drops it from
		// any artifact that doesn't install it.
		let emits_client = self.fullstack || self.target == Some(crate::platform::Target::Web);
		if emits_client && !self.rpc_endpoints.is_empty() {
			self.load_module("std/web/rpc", &mut visiting);
		}

		self.gate_by_reachability();

		if !self.diagnostics.is_empty() {
			Err(self.diagnostics.to_vec())
		} else {
			Ok(())
		}
	}

	// Enforce deploy-target tier gating by def-level
	// reachability. A forbidden-tier module (`std/sys/*` on `web`, `std/web/*`
	// on `sys`) is rejected only when reachable from an entry through
	// *non-`remote def`* code — a server island's server-only imports never
	// reach the client closure. This replaces the coarse per-`use` tier gate;
	// it runs after analysis and only when a deploy target is selected (the
	// ungated `run`/`test`/analyze flows skip it entirely).
	fn gate_by_reachability(&mut self) {
		let Some(target) = self.target else {
			return;
		};
		let roots = self.entry_modules.clone();
		self.gate_roots(&roots, target);
	}

	// FULLSTACK dual build: gate each artifact against its own target — the server
	// half (`entry_modules[0]`) as `Sys`, the client half (`[1]`) as `Web`. A single
	// `target` can't express the split (each side legitimately reaches the other
	// tier's stdlib through its own root), so the dual build skips the single-target
	// gate (`target` is `None`) and calls this instead, after `check()`.
	pub fn gate_fullstack(&mut self) -> Result<(), Vec<Diagnostic>> {
		if let [server, client] = self.entry_modules.clone().as_slice() {
			self.gate_roots(&[server.clone()], Target::Sys);
			self.gate_roots(&[client.clone()], Target::Web);
		}
		if self.diagnostics.is_empty() {
			Ok(())
		} else {
			Err(self.diagnostics.to_vec())
		}
	}

	// Reject any forbidden-tier module reachable from `roots` (through non-`remote
	// def` code — the island stop rule). Shared by the single-target gate and the
	// per-artifact fullstack gate.
	fn gate_roots(&mut self, roots: &[String], target: Target) {
		let mut reached: HashSet<String> = HashSet::new();
		let mut work: Vec<String> = roots.to_vec();
		// First importer of each followed module, for the diagnostic caret.
		let mut via: HashMap<String, (String, PathBuf, Range)> = HashMap::new();

		while let Some(name) = work.pop() {
			if !reached.insert(name.clone()) {
				continue;
			}
			let Some(module) = self.modules.get(&name) else {
				continue;
			};
			let Some(ast) = module.ast.as_ref() else {
				continue;
			};
			let importer_path = module.module_path.clone();
			// Follow only imports the module references outside `remote def`
			// bodies — the island stop rule.
			let live = crate::reachability::live_prefixes(ast);
			let follow: Vec<(String, Range)> = ast
				.uses
				.iter()
				.filter(|u| live.contains(&u.local_name().name))
				.map(|u| (u.module_name(), u.range))
				.collect();
			for (full, range) in follow {
				via
					.entry(full.clone())
					.or_insert_with(|| (name.clone(), importer_path.clone(), range));
				work.push(full);
			}
		}

		for module_name in &reached {
			let Some(message) = gate(Some(target), module_name) else {
				continue;
			};
			let mut diag = Diagnostic::error(message);
			if let Some((importer, path, range)) = via.get(module_name) {
				diag = diag
					.with_range(*range)
					.with_module(importer.clone(), path.clone());
			}
			self.diagnostics.push(diag);
		}
	}

	// Parse + analyze the synthetic prelude module. The source is baked
	// into the compiler binary so the language doesn't depend on a
	// stdlib install directory.
	fn load_prelude(&mut self) {
		const PRELUDE_SOURCE: &str = include_str!("prelude.pa");
		const NAME: &str = "__prelude__";
		// Already seeded (e.g. from a long-lived consumer's stdlib export
		// cache): the prelude's exports are immutable, so don't re-analyze.
		if self.exports_cache.contains_key(NAME) {
			return;
		}
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
		// if present, otherwise the last dotted segment — so `use sub/utils` binds
		// `utils` and `use sub/utils as u` binds `u`. The use-statement range spans
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
		let importer_is_test = is_test_module(module_name);
		let importer_path = self.modules.get(module_name).map(|m| m.module_path.clone());
		let mut rejected_imports: HashSet<String> = HashSet::new();
		for (full_name, _, range, use_range) in &imports {
			let _ = use_range;
			// Deploy-target tier gating (`std/sys/*` vs `std/web/*`) is *not*
			// enforced here at the `use` site — it's done after analysis by
			// def-level reachability (`gate_by_reachability`), so a `remote def`
			// body's server-only imports don't bar a web client. Only the
			// always-true structural rules (test-module and project-marker
			// imports) are rejected at the `use` site.
			let rejection: Option<(String, Range)> = if is_test_module(full_name) && !importer_is_test {
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
		// A test file (`foo.test`) importing the module it tests (`foo`) sees that
		// module's *private* surface too, so a unit test can reach its helpers
		// without exporting them just for testing. The link is the name: the test's
		// stem (`foo.test` → `foo`) names the module under test. The stem is matched
		// as a path *suffix* of the import — a test rooted at a subdirectory is named
		// relative to that root (`sys/static.test`) while it imports the module by its
		// full path (`std/sys/static`), so the stem is a suffix, not always equal.
		let test_stem: Option<&str> = if importer_is_test {
			module_name.strip_suffix(".test")
		} else {
			None
		};
		for (full_name, local_name, _, _) in &imports {
			if rejected_imports.contains(full_name) {
				continue;
			}
			if let Some(exports) = self.exports_cache.get(full_name) {
				let is_module_under_test = test_stem
					.is_some_and(|stem| full_name == stem || full_name.ends_with(&format!("/{stem}")));
				let exports = if is_module_under_test {
					exports.internal_view()
				} else {
					exports.clone()
				};
				imports_map.insert(local_name.clone(), exports);
				import_qualified.insert(local_name.clone(), full_name.clone());
			}
		}

		// Auto-imported modules: bound under a bare name in every user
		// module without an explicit `use`. Currently `std/ref` →
		// `ref`, `std/option` → `option`, `std/result` → `result`.
		// User code can shadow by binding the local name to something
		// else via `use`. Exports come from either a baked `.pa` source
		// (loaded via `load_module` into `exports_cache`) or from a
		// pre-registered native module. Auto-imports don't apply when
		// loading an auto-imported module itself — they'd otherwise
		// form a cycle among themselves (loading `std/option` would
		// recurse into loading `std/ref` etc. while `std/option` is
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

		// Incremental reuse: skip re-analysis when this module's source is
		// byte-identical to a prior clean analysis AND none of its
		// dependencies were reanalyzed this pass (a changed dependency could
		// shift this module's inferred signatures). An entry module is never
		// reused — its typed AST is the analysis result a caller reads, so it
		// must be freshly analyzed every pass. Only diagnostic-free analyses
		// are cached, so reuse yields exactly the diagnostics a full compile
		// would (a module with errors re-analyzes every pass).
		let source_hash = self
			.modules
			.get(module_name)
			.map(|m| m.source_hash)
			.unwrap_or(0);
		let is_entry = self.entry_modules.iter().any(|e| e == module_name);
		let deps_reanalyzed = imports
			.iter()
			.any(|(full, ..)| self.reanalyzed.contains(full));
		let reusable = !is_entry
			&& !deps_reanalyzed
			&& self
				.incremental
				.as_ref()
				.and_then(|c| c.get(module_name))
				.is_some_and(|e| e.source_hash == source_hash);
		if reusable {
			let exports = self.incremental.as_ref().unwrap()[module_name]
				.exports
				.clone();
			self.exports_cache.insert(module_name.to_string(), exports);
			visiting.remove(module_name);
			return;
		}

		// Analyze this module. The prelude's exports (enums, variant
		// constructors, instances) are implicitly available — pass them
		// in alongside explicit imports so name resolution + discharge
		// can use them.
		let prelude_exports = self.exports_cache.get("__prelude__").cloned();
		let hmr = self.hmr;
		// Snapshot the diagnostic count before the analyzer borrows the buffer:
		// a clean analysis (count unchanged) is the only kind we cache.
		let diag_before = self.diagnostics.len();
		let module = self.modules.get_mut(module_name).unwrap();
		let mut analyzer = Analyzer::new(&mut self.diagnostics);
		analyzer.set_imports(imports_map, import_qualified);
		analyzer.set_hmr(hmr);
		if let Some(exports) = prelude_exports {
			analyzer.add_imported_instances(&exports.instances);
			analyzer.set_prelude_exports(exports);
		}
		let _t = std::time::Instant::now();
		analyzer.analyze(module);
		timing_log(module_name, "analyze", _t.elapsed());

		// Collect any `remote def` endpoint metadata this module declared (with
		// resolved wire shapes + per-route fingerprints), for the lowerer to
		// synthesize client stubs / the dispatch table from. This is the
		// analyzer's last use, ending its `&mut self.diagnostics` borrow.
		let endpoint_meta = analyzer.take_endpoint_meta();
		let analysis_was_clean = self.diagnostics.len() == diag_before;
		self.reanalyzed.insert(module_name.to_string());
		self.rpc_endpoints.extend(endpoint_meta);

		// Cache its exports for any later importer (this pass), and — when the
		// analysis was clean — persist them for reuse on a later pass. Drop any
		// stale entry on a dirty analysis so its hash can't linger.
		if let Some(exports) = self
			.modules
			.get(module_name)
			.and_then(|m| m.exports.clone())
		{
			self
				.exports_cache
				.insert(module_name.to_string(), exports.clone());
			if let Some(cache) = self.incremental.as_mut() {
				if analysis_was_clean {
					cache.insert(
						module_name.to_string(),
						ModuleCacheEntry {
							source_hash,
							exports,
						},
					);
				} else {
					cache.remove(module_name);
				}
			}
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

/// Whether `name` is a discovered test *file* (`<segments>.test.pa`), as
/// opposed to ordinary code. These are import-gated: only other test modules may
/// `use` them. `std/test` is excluded — it's the test *framework* (stdlib),
/// importable from anywhere, and only incidentally shares the `.test` suffix.
fn is_test_module(name: &str) -> bool {
	name.ends_with(".test") && name != "std/test"
}

pub fn to_module_path(root_dir: &Path, module_name: &str) -> PathBuf {
	let mut path = root_dir.to_path_buf();
	// Test modules live in `<segments>.test.pa` files. Their module name keeps
	// the `.test` suffix (e.g. `foo/bar.test`), so we peel that off before
	// pushing the `/`-separated path segments.
	if let Some(stem) = module_name.strip_suffix(".test") {
		for segment in stem.split('/') {
			path.push(segment);
		}
		path.set_extension(format!("test.{}", FILE_EXTENSION));
	} else {
		for segment in module_name.split('/') {
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
						.replace(std::path::MAIN_SEPARATOR, "/");
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

	// Compile a synthetic `main` module under `target`, returning the
	// diagnostics (empty on success). The module source is fed in-memory, so no
	// disk access is needed; gated stdlib modules resolve from the baked sources.
	fn check_with(target: Option<Target>, source: &str) -> Vec<Diagnostic> {
		let mut compiler = Compiler::for_root_dir(std::env::temp_dir()).with_target(target);
		compiler.set_module_source("main".to_string(), source.as_bytes().to_vec());
		compiler.add_entry_module("main".to_string());
		match compiler.check() {
			Ok(()) => Vec::new(),
			Err(diags) => diags,
		}
	}

	#[test]
	fn sys_io_allowed_ungated_and_on_sys() {
		let src = "use std/sys/io\n\ndef main = fun { io.print \"hi\" }\n";
		assert!(check_with(None, src).is_empty());
		assert!(check_with(Some(Target::Sys), src).is_empty());
	}

	#[test]
	fn sys_io_rejected_on_web() {
		let src = "use std/sys/io\n\ndef main = fun { io.print \"hi\" }\n";
		let diags = check_with(Some(Target::Web), src);
		assert!(
			diags
				.iter()
				.any(|d| d.message.contains("std/sys/io") && d.message.contains("web")),
			"expected a web-target rejection for std/sys/io, got: {:?}",
			diags.iter().map(|d| &d.message).collect::<Vec<_>>()
		);
	}

	#[test]
	fn ungated_module_available_everywhere() {
		let src = "use std/list\n\ndef main = fun { list.length [1] }\n";
		for t in [None, Some(Target::Sys), Some(Target::Web)] {
			assert!(
				check_with(t, src).is_empty(),
				"std/list rejected on {:?}",
				t
			);
		}
	}

	// Compile several in-memory modules under `target` from `entry`.
	fn check_multi(target: Option<Target>, modules: &[(&str, &str)], entry: &str) -> Vec<Diagnostic> {
		let mut compiler = Compiler::for_root_dir(std::env::temp_dir()).with_target(target);
		for (name, src) in modules {
			compiler.set_module_source(name.to_string(), src.as_bytes().to_vec());
		}
		compiler.add_entry_module(entry.to_string());
		match compiler.check() {
			Ok(()) => Vec::new(),
			Err(diags) => diags,
		}
	}

	#[test]
	fn remote_def_body_imports_dont_reach_web_client() {
		// `api` touches std/sys/io only inside a `remote def` body — a server
		// island. A web client reaching `api`'s non-remote surface must not be
		// barred by that server-only import (it never enters the client closure).
		let api = "use std/task\nuse std/request\nuse std/sys/io\n\n\
			public def label :: fun nothing -> string = fun {\n\t\"api\"\n}\n\n\
			public remote def shout :: fun request string -> task string = fun _req msg {\n\
			\tlet _ = io.print msg\n\ttask.return msg\n}\n";
		let main = "use api\n\ndef main = fun {\n\tprint (api.label ())\n}\n";
		let diags = check_multi(Some(Target::Web), &[("api", api), ("main", main)], "main");
		assert!(
			diags.is_empty(),
			"a server-island import was wrongly barred on web: {:?}",
			diags.iter().map(|d| &d.message).collect::<Vec<_>>()
		);
	}

	#[test]
	fn client_reachable_sys_import_is_rejected_on_web() {
		// The control: when std/sys/io is reached through ordinary (non-remote)
		// code across a module boundary, it is correctly barred on web.
		let api = "use std/sys/io\n\n\
			public def announce :: fun nothing -> nothing = fun {\n\tio.print \"hi\"\n}\n";
		let main = "use api\n\ndef main = fun {\n\tapi.announce ()\n}\n";
		let diags = check_multi(Some(Target::Web), &[("api", api), ("main", main)], "main");
		assert!(
			diags.iter().any(|d| d.message.contains("std/sys/io")),
			"expected a web-target rejection for std/sys/io reached via a user module, got: {:?}",
			diags.iter().map(|d| &d.message).collect::<Vec<_>>()
		);
	}

	// FULLSTACK: check `modules` with `[server, client]` entries (target-less, so
	// `check`'s single-target gate is off), then run the per-artifact gate.
	fn gate_fullstack_multi(modules: &[(&str, &str)], server: &str, client: &str) -> Vec<Diagnostic> {
		let mut compiler = Compiler::for_root_dir(std::env::temp_dir()).with_fullstack(true);
		for (name, src) in modules {
			compiler.set_module_source(name.to_string(), src.as_bytes().to_vec());
		}
		compiler.add_entry_module(server.to_string());
		compiler.add_entry_module(client.to_string());
		if let Err(diags) = compiler.check() {
			return diags;
		}
		match compiler.gate_fullstack() {
			Ok(()) => Vec::new(),
			Err(diags) => diags,
		}
	}

	#[test]
	fn fullstack_gate_allows_each_tier_on_its_own_side() {
		// The server half reaches `std/sys/*`, the client half `std/web/*` — each
		// legal on its own artifact. A single gating profile couldn't admit both.
		let server = "use std/sys/io\n\ndef main = fun {\n\tio.print \"s\"\n}\n";
		let client = "use std/web/dom\n\ndef main = fun {\n\tlet _ = dom.body ()\n\t()\n}\n";
		let diags = gate_fullstack_multi(
			&[("server", server), ("client", client)],
			"server",
			"client",
		);
		assert!(
			diags.is_empty(),
			"fullstack gate wrongly barred a tier on its own side: {:?}",
			diags.iter().map(|d| &d.message).collect::<Vec<_>>()
		);
	}

	#[test]
	fn fullstack_gate_bars_a_sys_leak_into_the_client() {
		// The same `std/sys/io` import: fine reached from the server root, barred
		// from the client root — the per-artifact split in action.
		let server = "use std/sys/io\n\ndef main = fun {\n\tio.print \"s\"\n}\n";
		let client = "use std/sys/io\n\ndef main = fun {\n\tio.print \"c\"\n}\n";
		let diags = gate_fullstack_multi(
			&[("server", server), ("client", client)],
			"server",
			"client",
		);
		assert!(
			diags
				.iter()
				.any(|d| d.message.contains("std/sys/io") && d.message.contains("web")),
			"expected the client root to bar std/sys/io on web, got: {:?}",
			diags.iter().map(|d| &d.message).collect::<Vec<_>>()
		);
	}
}

// A test file (`foo.test`) sees the private surface of the module it tests
// (`foo`), so a unit test can reach a module's helpers without exporting them
// just for testing. Every other importer still sees only the public surface.
#[cfg(test)]
mod test_sibling_visibility_tests {
	use super::*;

	fn check_multi(modules: &[(&str, &str)], entry: &str) -> Vec<Diagnostic> {
		let mut compiler = Compiler::for_root_dir(std::env::temp_dir());
		for (name, src) in modules {
			compiler.set_module_source(name.to_string(), src.as_bytes().to_vec());
		}
		compiler.add_entry_module(entry.to_string());
		match compiler.check() {
			Ok(()) => Vec::new(),
			Err(diags) => diags,
		}
	}

	fn errors(diags: &[Diagnostic]) -> Vec<&String> {
		diags
			.iter()
			.filter(|d| d.is_error())
			.map(|d| &d.message)
			.collect()
	}

	// A module exposing a non-public surface of every kind: a private value, a
	// private enum, an `opaque` enum (type public, constructors hidden), a
	// private alias, and a private trait + instance.
	const FOO: &str = "\
def secret-add :: fun int int -> int = fun a b {\n\
\ta + b\n\
}\n\
\n\
enum shade {\n\
\tdark\n\
\tlight\n\
}\n\
\n\
opaque enum tag {\n\
\ttag-of int\n\
}\n\
\n\
alias pt {\n\
\tx :: int,\n\
\ty :: int,\n\
}\n\
\n\
trait my-show a {\n\
\tmy-show :: fun a -> string\n\
}\n\
\n\
implement my-show int {\n\
\tdef my-show = fun n {\n\
\t\tto-string n\n\
\t}\n\
}\n";

	#[test]
	fn sibling_test_sees_every_private_kind() {
		// The sibling test reaches the private value, constructs and matches the
		// private enum, constructs the `opaque` enum's hidden constructor, builds
		// the private alias, and dispatches the private trait method.
		let foo_test = "\
use foo\n\
\n\
def use-value :: fun int -> int = fun x {\n\
\tfoo.secret-add x 1\n\
}\n\
\n\
def make-enum :: fun nothing -> foo.shade = fun {\n\
\tfoo.shade.dark\n\
}\n\
\n\
def match-enum :: fun foo.shade -> int = fun s {\n\
\twhen s is foo.shade.dark {\n\
\t\t0\n\
\t} is foo.shade.light {\n\
\t\t1\n\
\t}\n\
}\n\
\n\
def make-opaque :: fun nothing -> foo.tag = fun {\n\
\tfoo.tag.tag-of 5\n\
}\n\
\n\
def make-alias :: fun nothing -> foo.pt = fun {\n\
\tfoo.pt { x: 1, y: 2 }\n\
}\n\
\n\
def call-trait :: fun nothing -> string = fun {\n\
\tmy-show 7\n\
}\n";
		let diags = check_multi(&[("foo", FOO), ("foo.test", foo_test)], "foo.test");
		assert!(
			errors(&diags).is_empty(),
			"a sibling test was denied access to its module's private surface: {:?}",
			errors(&diags)
		);
	}

	#[test]
	fn non_test_importer_still_cannot_reach_private() {
		let client = "use foo\n\ndef out :: int = foo.secret-add 2 3\n";
		let diags = check_multi(&[("foo", FOO), ("client", client)], "client");
		assert!(
			diags
				.iter()
				.any(|d| d.message.contains("secret-add") && d.message.contains("private")),
			"expected `secret-add` to stay private to a non-test importer, got: {:?}",
			errors(&diags)
		);
	}

	#[test]
	fn unrelated_test_module_cannot_reach_private() {
		// `other.test` is a test file, but it doesn't test `foo` (its stem is
		// `other`, not `foo`), so it sees only `foo`'s public surface.
		let other_test = "use foo\n\ndef out :: int = foo.secret-add 2 3\n";
		let diags = check_multi(&[("foo", FOO), ("other.test", other_test)], "other.test");
		assert!(
			diags
				.iter()
				.any(|d| d.message.contains("secret-add") && d.message.contains("private")),
			"expected `secret-add` to stay private to a non-sibling test, got: {:?}",
			errors(&diags)
		);
	}
}
