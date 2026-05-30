use compiler::ast::*;
use compiler::{Diagnostic, Module, Range, find_project_root, to_module_path};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// Go-to-definition, resolved from parser output alone — the same model as
// `semantic_tokens`. We deliberately skip the full analyzer: it needs the
// project's imports resolved from disk and produces a `!Send` `Module`,
// whereas the syntactic structure is enough to resolve definitions.
//
// Same-file resolution is two halves: collect every binding (with the
// source region it's visible in) and every reference, then for the
// reference under the cursor pick the binding with the matching name whose
// scope contains the cursor, preferring the innermost (smallest) scope so
// shadowing resolves correctly.
//
// Cross-module: a `module.symbol` access (or a `use` path) resolves the
// imported module to a file on disk, parses it, and finds the top-level
// def there. stdlib/native modules have no project file, so those simply
// don't resolve.

/// Where a definition lives, as a navigable file location (the goto result).
pub enum Target {
	/// In the file under the cursor.
	Here(Range),
	/// In another module's file.
	OtherFile { path: PathBuf, range: Range },
}

/// The raw resolution, before turning a cross-module hit into a navigable
/// file. Hover reuses this to read a definition's doc without materializing
/// anything to disk.
pub enum Resolved {
	Here(Range),
	OtherModule {
		// The parsed target module (for reading docs).
		module: Module,
		// The def's name range within it.
		range: Range,
		// How to produce a navigable file for it.
		location: ModuleLocation,
	},
}

/// Where an imported module's source lives.
pub enum ModuleLocation {
	/// A user module: a real file on disk.
	Disk(PathBuf),
	/// A baked-in stdlib module (source inlined in the compiler binary),
	/// named e.g. `core.list`. Materialized to a cache file on demand.
	Stdlib(String),
}

/// What a reference resolves against. A bare value identifier may actually
/// name a (payload-less) enum variant, so `Value` falls back to the variant
/// table when no value binding matches.
#[derive(Clone, Copy, PartialEq)]
enum RefKind {
	Value,
	Type,
	Variant,
	Namespace,
}

struct Reference {
	range: Range,
	name: String,
	kind: RefKind,
}

// A `module.symbol` access: the receiver names an imported namespace, the
// field/type names a top-level def in that module. Resolved by loading the
// imported module's file.
struct QualifiedRef {
	range: Range,
	// Local namespace name (the receiver), e.g. `colors` in `colors.color`.
	namespace: String,
	// The symbol named after the dot.
	name: String,
	// True in type position (`module.Type`), so we prefer a type def.
	is_type: bool,
}

struct Binding {
	name: String,
	// Where to jump: the identifier's own range.
	def_range: Range,
	// The region the binding is visible in. `None` means module-global
	// (top-level defs, enums, variants, imports) — always in scope.
	scope: Option<Range>,
}

/// Resolve the identifier under the cursor to a navigable definition. `path`
/// is the document's own file path, used to locate imported modules. For a
/// stdlib module the source is materialized to a cache file so the editor
/// has something to open.
pub fn goto_definition(source: &[u8], path: &Path, line: u32, character: u32) -> Option<Target> {
	goto_definition_in(source, path, line, character, None)
}

// Inner `goto_definition` with an injectable stdlib-cache root: `cache_root`
// overrides where stdlib sources materialize (tests pass a temp dir); `None`
// uses the OS cache directory. Keeping the override a parameter rather than an
// env var means tests don't mutate process-global state and can't race.
fn goto_definition_in(
	source: &[u8],
	path: &Path,
	line: u32,
	character: u32,
	cache_root: Option<&Path>,
) -> Option<Target> {
	match resolve(source, path, line, character)? {
		Resolved::Here(range) => Some(Target::Here(range)),
		Resolved::OtherModule {
			range, location, ..
		} => {
			let file = match location {
				ModuleLocation::Disk(p) => p,
				ModuleLocation::Stdlib(name) => stdlib_cache_path(&name, cache_root)?,
			};
			Some(Target::OtherFile { path: file, range })
		}
	}
}

/// Resolve the identifier under the cursor, without materializing anything.
/// Returns the parsed target module for cross-module hits so callers can
/// read docs from it.
pub fn resolve(source: &[u8], path: &Path, line: u32, character: u32) -> Option<Resolved> {
	let mut module = Module::new("<lsp>".to_string(), PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);
	let ast = module.ast.as_ref()?;

	let line = line as usize;
	let character = character as usize;

	// A `use` path/alias under the cursor jumps to the imported module.
	for u in &ast.uses {
		let on_path = u
			.path
			.iter()
			.any(|seg| contains(&seg.range, line, character));
		let on_alias = u
			.alias
			.as_ref()
			.is_some_and(|a| contains(&a.range, line, character));
		if on_path || on_alias {
			let (module, location) = load_imported_module(&u.module_name(), path)?;
			return Some(Resolved::OtherModule {
				module,
				range: Range::collapsed(0, 0),
				location,
			});
		}
	}

	let mut r = Resolver::new(ast);
	r.walk_module(ast);

	// A `module.symbol` access under the cursor resolves into the other module.
	if let Some(q) = r
		.qualified
		.iter()
		.filter(|q| contains(&q.range, line, character))
		.min_by_key(|q| range_size(&q.range))
	{
		return resolve_cross_module(&ast.uses, path, q);
	}

	// The reference under the cursor: smallest range that contains it.
	let reference = r
		.refs
		.iter()
		.filter(|rf| contains(&rf.range, line, character))
		.min_by_key(|rf| range_size(&rf.range))?;

	let range = match reference.kind {
		RefKind::Value => {
			// Values shadow into variants: a payload-less variant used as a
			// value (`none`) parses as a plain identifier.
			resolve_binding(&r.values, &reference.name, line, character)
				.or_else(|| resolve_binding(&r.variants, &reference.name, line, character))
		}
		RefKind::Type => resolve_binding(&r.types, &reference.name, line, character),
		RefKind::Variant => resolve_binding(&r.variants, &reference.name, line, character),
		RefKind::Namespace => resolve_binding(&r.namespaces, &reference.name, line, character),
	};
	range.map(Resolved::Here)
}

fn resolve_binding(table: &[Binding], name: &str, line: usize, character: usize) -> Option<Range> {
	table
		.iter()
		.filter(|b| b.name == name && scope_contains(&b.scope, line, character))
		.min_by_key(|b| scope_size(&b.scope))
		.map(|b| b.def_range)
}

// Map a local namespace name to its imported module via the `use` list, load
// that module, and find the top-level def named by the access.
fn resolve_cross_module(uses: &[UseNode], current: &Path, q: &QualifiedRef) -> Option<Resolved> {
	let full_name = uses
		.iter()
		.find(|u| u.local_name().name == q.namespace)
		.map(|u| u.module_name())?;

	let (module, location) = load_imported_module(&full_name, current)?;
	let range = find_top_level_def(module.ast.as_ref()?, &q.name, q.is_type)?;
	Some(Resolved::OtherModule {
		module,
		range,
		location,
	})
}

/// Load and parse an imported module by its fully-qualified name, relative to
/// `current` (the importing file). Exposed so hover can read a module's own
/// top-level doc comment without re-deriving the stdlib/disk resolution order.
pub fn imported_module(module_name: &str, current: &Path) -> Option<Module> {
	load_imported_module(module_name, current).map(|(m, _)| m)
}

// Load and parse an imported module. Baked-in stdlib source takes precedence
// over a same-named file on disk, matching the compiler's own load order.
// Returns `None` for a module with no source we can find.
fn load_imported_module(module_name: &str, current: &Path) -> Option<(Module, ModuleLocation)> {
	let mut diagnostics: Vec<Diagnostic> = Vec::new();

	if let Some(source) = compiler::lookup_stdlib_source(module_name) {
		let mut module = Module::new(
			module_name.to_string(),
			PathBuf::from(format!("<stdlib:{}>", module_name)),
		);
		module.parse_from_bytes(source.as_bytes().to_vec(), &mut diagnostics);
		module.ast.as_ref()?;
		return Some((module, ModuleLocation::Stdlib(module_name.to_string())));
	}

	// User module: resolve its file relative to the project root (nearest
	// `pluma.pa`), falling back to the current file's directory.
	let root = find_project_root(current).or_else(|| current.parent().map(|p| p.to_path_buf()))?;
	let path = to_module_path(&root, module_name);
	let bytes = std::fs::read(&path).ok()?;
	let mut module = Module::new(module_name.to_string(), path.clone());
	module.parse_from_bytes(bytes, &mut diagnostics);
	module.ast.as_ref()?;
	Some((module, ModuleLocation::Disk(path)))
}

// -- stdlib materialization -----------------------------------------------

// Write the inlined stdlib tree to a versioned cache directory (once) and
// return the file path for one module. The whole tree plus a `pluma.pa`
// marker is written so intra-stdlib `use`s resolve and the opened file
// analyzes cleanly. Versioning the path means a newer compiler refreshes it.
fn stdlib_cache_path(module_name: &str, cache_root: Option<&Path>) -> Option<PathBuf> {
	let base = match cache_root {
		Some(b) => b.to_path_buf(),
		None => cache_base()?,
	};
	let root = base.join("pluma").join(compiler::VERSION).join("stdlib");

	std::fs::create_dir_all(&root).ok()?;
	let marker = root.join(compiler::PROJECT_MARKER_FILE);
	if !marker.exists() {
		std::fs::write(&marker, "").ok()?;
	}
	for (name, source) in compiler::stdlib_sources() {
		let path = to_module_path(&root, name);
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).ok()?;
		}
		// Versioned dir already isolates compiler versions; write once.
		if !path.exists() {
			write_readonly(&path, source).ok()?;
		}
	}

	let path = to_module_path(&root, module_name);
	path.is_file().then_some(path)
}

// Write a file and mark it read-only: the materialized stdlib is a view of
// the compiler's inlined source, not something to edit (edits wouldn't feed
// back). Deletion still works — on Unix it depends on the parent dir, and a
// version bump writes a fresh tree elsewhere.
fn write_readonly(path: &Path, contents: &str) -> std::io::Result<()> {
	std::fs::write(path, contents)?;
	let mut perms = std::fs::metadata(path)?.permissions();
	perms.set_readonly(true);
	std::fs::set_permissions(path, perms)
}

// The OS user cache directory: `$XDG_CACHE_HOME`, else the platform default.
fn cache_base() -> Option<PathBuf> {
	if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
		return Some(PathBuf::from(xdg));
	}
	#[cfg(target_os = "macos")]
	let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"));
	#[cfg(target_os = "windows")]
	let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
	#[cfg(not(any(target_os = "macos", target_os = "windows")))]
	let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"));
	base
}

// Find a top-level def by name. When `prefer_type`, a type def (enum / alias
// / trait) wins over a value of the same name; otherwise a value wins. Falls
// back to any def with the name so e.g. `module.color` (an enum referenced in
// value position) still resolves.
fn find_top_level_def(ast: &ModuleNode, name: &str, prefer_type: bool) -> Option<Range> {
	let mut preferred: Option<Range> = None;
	let mut fallback: Option<Range> = None;
	for def in &ast.body {
		if def.name.name != name {
			continue;
		}
		let is_type = matches!(
			def.kind,
			DefinitionKind::Alias(_) | DefinitionKind::Enum(_) | DefinitionKind::Trait(_)
		);
		if is_type == prefer_type {
			preferred.get_or_insert(def.name.range);
		} else {
			fallback.get_or_insert(def.name.range);
		}
	}
	preferred.or(fallback)
}

// -- collection -----------------------------------------------------------

struct Resolver {
	module_names: HashSet<String>,
	enum_names: HashSet<String>,
	// Every variant name declared in the file. A nullary variant pattern
	// (`when o is none`) parses as a plain identifier — indistinguishable
	// from a fresh binding without analysis — so we lean on this set to
	// classify it as a variant reference instead.
	variant_names: HashSet<String>,
	values: Vec<Binding>,
	types: Vec<Binding>,
	variants: Vec<Binding>,
	namespaces: Vec<Binding>,
	refs: Vec<Reference>,
	qualified: Vec<QualifiedRef>,
}

impl Resolver {
	fn new(ast: &ModuleNode) -> Self {
		let mut module_names = HashSet::new();
		for u in &ast.uses {
			module_names.insert(u.local_name().name.clone());
		}
		let mut enum_names = HashSet::new();
		let mut variant_names = HashSet::new();
		for def in &ast.body {
			if let DefinitionKind::Enum(en) = &def.kind {
				enum_names.insert(def.name.name.clone());
				for v in &en.variants {
					variant_names.insert(v.name.name.clone());
				}
			}
		}
		Self {
			module_names,
			enum_names,
			variant_names,
			values: Vec::new(),
			types: Vec::new(),
			variants: Vec::new(),
			namespaces: Vec::new(),
			refs: Vec::new(),
			qualified: Vec::new(),
		}
	}

	fn bind_value(&mut self, id: &IdentifierNode, scope: Option<Range>) {
		self.values.push(Binding {
			name: id.name.clone(),
			def_range: id.range,
			scope,
		});
	}

	fn reference(&mut self, id: &IdentifierNode, kind: RefKind) {
		self.refs.push(Reference {
			range: id.range,
			name: id.name.clone(),
			kind,
		});
	}

	fn walk_module(&mut self, m: &ModuleNode) {
		for u in &m.uses {
			// The local name (alias or last path segment) is the namespace
			// binding; jumping to it lands on the import.
			let local = u.local_name();
			self.namespaces.push(Binding {
				name: local.name.clone(),
				def_range: local.range,
				scope: None,
			});
		}
		for def in &m.body {
			self.walk_def(def);
		}
	}

	fn walk_def(&mut self, d: &DefinitionNode) {
		match &d.kind {
			DefinitionKind::Expr(expr) => {
				self.bind_value(&d.name, None);
				if let Some(ann) = &d.type_annotation {
					self.walk_type_expr(ann, None);
				}
				self.walk_expr(expr, None);
			}
			DefinitionKind::Alias(ty_expr) => {
				self.types.push(Binding {
					name: d.name.name.clone(),
					def_range: d.name.range,
					scope: None,
				});
				self.walk_type_expr(ty_expr, None);
			}
			DefinitionKind::Enum(en) => {
				self.types.push(Binding {
					name: d.name.name.clone(),
					def_range: d.name.range,
					scope: None,
				});
				for p in &en.params {
					self.types.push(Binding {
						name: p.name.clone(),
						def_range: p.range,
						scope: Some(en.range),
					});
				}
				for v in &en.variants {
					self.variants.push(Binding {
						name: v.name.name.clone(),
						def_range: v.name.range,
						scope: None,
					});
					if let Some(params) = &v.params {
						for p in params {
							self.walk_type_expr(p, Some(en.range));
						}
					}
				}
			}
			DefinitionKind::Trait(t) => {
				self.types.push(Binding {
					name: d.name.name.clone(),
					def_range: d.name.range,
					scope: None,
				});
				self.types.push(Binding {
					name: t.param.name.clone(),
					def_range: t.param.range,
					scope: Some(t.range),
				});
				for m in &t.methods {
					// Trait methods are callable as bare values (`add`), so
					// register them as value bindings too.
					self.bind_value(&m.name, None);
					self.walk_type_expr(&m.signature, Some(t.range));
					if let Some(default) = &m.default {
						self.walk_expr(default, Some(t.range));
					}
				}
			}
			DefinitionKind::Instance(inst) => {
				self.reference(&inst.trait_name, RefKind::Type);
				self.walk_type_expr(&inst.head, Some(inst.range));
				for c in &inst.where_clause {
					self.reference(&c.trait_name, RefKind::Type);
				}
				for method in &inst.methods {
					self.walk_def(method);
				}
			}
		}
	}

	fn walk_expr(&mut self, e: &ExprNode, scope: Option<Range>) {
		match &e.kind {
			ExprKind::Identifier(id) => {
				let kind = if self.module_names.contains(&id.name) {
					RefKind::Namespace
				} else {
					RefKind::Value
				};
				self.reference(id, kind);
			}
			ExprKind::Literal(_) | ExprKind::Regex(_) | ExprKind::EmptyTuple | ExprKind::Builtin(_) => {}
			ExprKind::BinaryOperation { left, right, .. } => {
				self.walk_expr(left, scope);
				self.walk_expr(right, scope);
			}
			ExprKind::UnaryOperation { right, .. } => self.walk_expr(right, scope),
			ExprKind::ElementAccess { receiver, .. } => self.walk_expr(receiver, scope),
			ExprKind::FieldAccess { receiver, field } => {
				match &receiver.kind {
					ExprKind::Identifier(id) if self.module_names.contains(&id.name) => {
						// `module.value`: the namespace resolves to its import; the
						// field resolves cross-module into the other file.
						self.reference(id, RefKind::Namespace);
						self.qualified.push(QualifiedRef {
							range: field.range,
							namespace: id.name.clone(),
							name: field.name.clone(),
							is_type: false,
						});
					}
					ExprKind::Identifier(id) if self.enum_names.contains(&id.name) => {
						// `enum.variant`: receiver is the enum type, field the variant.
						self.reference(id, RefKind::Type);
						self.reference(field, RefKind::Variant);
					}
					_ => {
						// A record field access — receiver is an expression, the
						// field is a structural label with no definition site.
						self.walk_expr(receiver, scope);
					}
				}
			}
			ExprKind::Fun(f) => self.walk_fun(f, scope),
			ExprKind::Call(c) => {
				match &c.callee.kind {
					ExprKind::Identifier(id) => {
						let kind = if self.module_names.contains(&id.name) {
							RefKind::Namespace
						} else {
							RefKind::Value
						};
						self.reference(id, kind);
					}
					_ => self.walk_expr(&c.callee, scope),
				}
				for arg in &c.args {
					self.walk_expr(arg, scope);
				}
			}
			ExprKind::Grouping(inner) | ExprKind::Defer(inner) => self.walk_expr(inner, scope),
			ExprKind::Interpolation(parts) | ExprKind::Tuple(parts) => {
				for p in parts {
					self.walk_expr(p, scope);
				}
			}
			ExprKind::List(items) => {
				for item in items {
					self.walk_expr(item.expr(), scope);
				}
			}
			ExprKind::Let(l) => {
				// The binding is visible for the rest of the enclosing scope.
				self.bind_pattern(&l.pattern, scope);
				if let Some(ann) = &l.type_annotation {
					self.walk_type_expr(ann, scope);
				}
				self.walk_expr(&l.value, scope);
			}
			ExprKind::Record(fields) => {
				for (_, value) in fields {
					self.walk_expr(value, scope);
				}
			}
			ExprKind::RecordUpdate { base, fields } => {
				self.walk_expr(base, scope);
				for (_, value) in fields {
					self.walk_expr(value, scope);
				}
			}
			ExprKind::If(i) => {
				let inner = Some(i.range);
				self.walk_expr(&i.subject, scope);
				self.bind_pattern(&i.pattern, inner);
				for e in &i.body {
					self.walk_expr(e, inner);
				}
				if let Some(else_body) = &i.else_body {
					for e in else_body {
						self.walk_expr(e, inner);
					}
				}
			}
			ExprKind::When(w) => {
				self.walk_expr(&w.subject, scope);
				for case in &w.cases {
					let inner = Some(case.range);
					self.bind_pattern(&case.pattern, inner);
					for e in &case.body {
						self.walk_expr(e, inner);
					}
				}
			}
			ExprKind::While(w) => {
				let inner = Some(w.range);
				self.walk_expr(&w.subject, scope);
				self.bind_pattern(&w.pattern, inner);
				for e in &w.body {
					self.walk_expr(e, inner);
				}
			}
			ExprKind::Scope(s) => {
				let inner = Some(s.range);
				for e in &s.body {
					self.walk_expr(e, inner);
				}
			}
			ExprKind::Try(t) => {
				let inner = Some(t.range);
				self.bind_pattern(&t.pattern, inner);
				self.walk_expr(&t.value, scope);
				for e in &t.rest {
					self.walk_expr(e, inner);
				}
			}
			ExprKind::NamespaceAccess(_) => {
				// Parser output never carries NamespaceAccess (the analyzer
				// builds it), so there's nothing to do here.
			}
		}
	}

	fn walk_fun(&mut self, f: &FunNode, _outer: Option<Range>) {
		let inner = Some(f.range);
		for p in &f.params {
			self.bind_value(&p.ident, inner);
		}
		for e in &f.body {
			self.walk_expr(e, inner);
		}
	}

	// Pattern identifiers introduce bindings; constructor names are
	// references to variants.
	fn bind_pattern(&mut self, p: &PatternNode, scope: Option<Range>) {
		match &p.kind {
			PatternKind::Identifier(id) => {
				// A nullary variant (`is none`) parses as an identifier; if the
				// name is a known variant, treat it as a reference, not a binding.
				if self.variant_names.contains(&id.name) {
					self.reference(id, RefKind::Variant);
				} else {
					self.bind_value(id, scope);
				}
			}
			PatternKind::Constructor(name, inner) => {
				self.reference(name, RefKind::Variant);
				for ip in inner {
					self.bind_pattern(ip, scope);
				}
			}
			PatternKind::Tuple(items) => {
				for ip in items {
					self.bind_pattern(ip, scope);
				}
			}
			PatternKind::Record { fields, rest } => {
				for (_, sub) in fields {
					self.bind_pattern(sub, scope);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						self.bind_value(name, scope);
					}
				}
			}
			PatternKind::List { items, rest } => {
				for ip in items {
					self.bind_pattern(ip, scope);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						self.bind_value(name, scope);
					}
				}
			}
			PatternKind::Interpolation(parts) => {
				for e in parts {
					self.walk_expr(e, scope);
				}
			}
			PatternKind::Underscore | PatternKind::Literal(_) => {}
		}
	}

	fn walk_type_expr(&mut self, t: &TypeExprNode, scope: Option<Range>) {
		match &t.kind {
			TypeExprKind::Single(id) => {
				match &id.module {
					Some(module) => {
						// `module.Type`: the prefix is a namespace; the type name
						// resolves cross-module. The name sits right after the dot,
						// past the module prefix.
						self.reference(module, RefKind::Namespace);
						let name_col = module.range.end.col + 1;
						self.qualified.push(QualifiedRef {
							range: Range::within_line(module.range.end.line, name_col, name_col + id.name.len()),
							namespace: module.name.clone(),
							name: id.name.clone(),
							is_type: true,
						});
					}
					None => {
						// `id.range` covers the whole name; emit a type reference
						// against it. Built-ins (`int`, `string`, …) simply won't
						// resolve, which is the right outcome.
						self.refs.push(Reference {
							range: id.range,
							name: id.name.clone(),
							kind: RefKind::Type,
						});
						let _ = scope;
					}
				}
				for g in &id.generics {
					self.walk_type_expr(g, scope);
				}
			}
			TypeExprKind::Func(params, ret) => {
				for p in params {
					self.walk_type_expr(p, scope);
				}
				self.walk_type_expr(ret, scope);
			}
			TypeExprKind::Tuple(items) => {
				for it in items {
					self.walk_type_expr(it, scope);
				}
			}
			TypeExprKind::Record(fields) => {
				for (_, ty) in fields {
					self.walk_type_expr(ty, scope);
				}
			}
			TypeExprKind::EmptyTuple | TypeExprKind::Grouping(_) => {
				if let TypeExprKind::Grouping(inner) = &t.kind {
					self.walk_type_expr(inner, scope);
				}
			}
		}
	}
}

// -- range helpers --------------------------------------------------------

fn contains(r: &Range, line: usize, character: usize) -> bool {
	if line < r.start.line || line > r.end.line {
		return false;
	}
	if line == r.start.line && character < r.start.col {
		return false;
	}
	if line == r.end.line && character > r.end.col {
		return false;
	}
	true
}

fn scope_contains(scope: &Option<Range>, line: usize, character: usize) -> bool {
	match scope {
		None => true,
		Some(r) => contains(r, line, character),
	}
}

fn range_size(r: &Range) -> usize {
	let lines = r.end.line.saturating_sub(r.start.line);
	let cols = if r.start.line == r.end.line {
		r.end.col.saturating_sub(r.start.col)
	} else {
		r.end.col
	};
	lines * 100_000 + cols
}

fn scope_size(scope: &Option<Range>) -> usize {
	match scope {
		None => usize::MAX,
		Some(r) => range_size(r),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Resolve a same-file definition at (line, col), returned as
	// (start_line, start_col). Non-`Here` targets count as no match.
	fn goto(src: &str, line: u32, col: u32) -> Option<(usize, usize)> {
		match goto_definition(src.as_bytes(), &PathBuf::new(), line, col) {
			Some(Target::Here(r)) => Some((r.start.line, r.start.col)),
			_ => None,
		}
	}

	// Lay down a throwaway project (a `pluma.pa` marker + the given files)
	// under the temp dir, keyed by `name` so parallel tests don't collide.
	fn temp_project(name: &str, files: &[(&str, &str)]) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("pluma-goto-{}-{}", name, std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("pluma.pa"), "").unwrap();
		for (file, contents) in files {
			std::fs::write(dir.join(file), contents).unwrap();
		}
		dir
	}

	#[test]
	fn local_param_use_jumps_to_param() {
		// `name` used on line 1 resolves to the param on line 0.
		let src = "def greet = fun name {\n\tprint name\n}\n";
		assert_eq!(goto(src, 1, 7), Some((0, 16)));
	}

	#[test]
	fn top_level_def_reference() {
		let src = "def helper = fun { 1 }\ndef main = fun {\n\thelper ()\n}\n";
		// `helper` call on line 2 jumps to its def on line 0 (col 4).
		assert_eq!(goto(src, 2, 1), Some((0, 4)));
	}

	#[test]
	fn let_binding_use() {
		let src = "def main = fun {\n\tlet x = 42\n\tprint x\n}\n";
		// `x` on line 2 resolves to the let binding on line 1 (col 5).
		assert_eq!(goto(src, 2, 7), Some((1, 5)));
	}

	#[test]
	fn shadowing_prefers_innermost() {
		// A param `x` shadows a top-level `def x`. The reference inside the
		// fun must resolve to the param, not the global.
		let src = "def x = 1\ndef f = fun x {\n\tprint x\n}\n";
		assert_eq!(goto(src, 2, 7), Some((1, 12)));
	}

	#[test]
	fn enum_variant_qualified_access() {
		let src = "enum color {\n\tred\n\tgreen\n}\ndef c = color.red\n";
		// `red` in `color.red` jumps to the variant decl on line 1 (col 1).
		assert_eq!(goto(src, 4, 14), Some((1, 1)));
		// `color` receiver jumps to the enum decl on line 0 (col 5).
		assert_eq!(goto(src, 4, 8), Some((0, 5)));
	}

	#[test]
	fn variant_pattern_constructor() {
		let src =
			"enum opt {\n\tsome\n\tnone\n}\ndef f = fun o {\n\twhen o is some { 1 } is none { 0 }\n}\n";
		// `some` in the pattern jumps to its variant decl (line 1, col 1).
		assert_eq!(goto(src, 5, 12), Some((1, 1)));
	}

	#[test]
	fn use_import_namespace() {
		let src = "use core.math\n\ndef x = math.pi\n";
		// `math` in `math.pi` jumps to the import's local name on line 0.
		// `use core.math` — `math` starts at col 9.
		assert_eq!(goto(src, 2, 8), Some((0, 9)));
	}

	#[test]
	fn type_reference_to_alias() {
		// Alias syntax is `alias NAME TYPE` (no `=`).
		let src = "alias my-int int\ndef x :: my-int = 5\n";
		// `my-int` in the annotation (line 1, col 9) jumps to the alias decl
		// on line 0 (col 6).
		assert_eq!(goto(src, 1, 9), Some((0, 6)));
	}

	#[test]
	fn unresolved_returns_none() {
		// `int` is a built-in with no definition site in the file.
		let src = "def x :: int = 5\n";
		assert_eq!(goto(src, 0, 9), None);
		// Whitespace / punctuation resolves to nothing.
		let src2 = "def x = 1\n";
		assert_eq!(goto(src2, 0, 3), None);
	}

	#[test]
	fn cross_module_value_jumps_into_other_file() {
		let dir = temp_project(
			"value",
			&[(
				"colors.pa",
				"enum color {\n\tred\n}\ndef helper = fun { 1 }\n",
			)],
		);
		let main = "use colors\n\ndef x = colors.helper ()\n";
		let main_path = dir.join("main.pa");
		// `helper` in `colors.helper` is at line 2, col 16.
		match goto_definition(main.as_bytes(), &main_path, 2, 16) {
			Some(Target::OtherFile { path, range }) => {
				assert!(path.ends_with("colors.pa"), "path: {:?}", path);
				// `def helper` is on line 3, col 4.
				assert_eq!((range.start.line, range.start.col), (3, 4));
			}
			other => panic!("expected OtherFile, got {}", target_kind(&other)),
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn cross_module_type_jumps_into_other_file() {
		let dir = temp_project("type", &[("colors.pa", "enum color {\n\tred\n}\n")]);
		let main = "use colors\n\nalias t colors.color\n";
		let main_path = dir.join("main.pa");
		// `color` in `colors.color` (type position) is at line 2, col 16.
		match goto_definition(main.as_bytes(), &main_path, 2, 16) {
			Some(Target::OtherFile { path, range }) => {
				assert!(path.ends_with("colors.pa"), "path: {:?}", path);
				// `enum color` is on line 0, col 5.
				assert_eq!((range.start.line, range.start.col), (0, 5));
			}
			other => panic!("expected OtherFile, got {}", target_kind(&other)),
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn use_path_jumps_to_module_file() {
		let dir = temp_project("usepath", &[("colors.pa", "enum color {\n\tred\n}\n")]);
		let main = "use colors\n\ndef x = 1\n";
		let main_path = dir.join("main.pa");
		// `colors` in the `use` path is at line 0, col 4-9.
		match goto_definition(main.as_bytes(), &main_path, 0, 5) {
			Some(Target::OtherFile { path, range }) => {
				assert!(path.ends_with("colors.pa"), "path: {:?}", path);
				assert_eq!((range.start.line, range.start.col), (0, 0));
			}
			other => panic!("expected OtherFile, got {}", target_kind(&other)),
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn unknown_module_resolves_to_nothing() {
		// A module that's neither stdlib nor an on-disk file can't resolve.
		let dir = temp_project("unknown", &[]);
		let main = "use whatever\n\ndef x = whatever.foo ()\n";
		let main_path = dir.join("main.pa");
		// `foo` in `whatever.foo` is at line 2, col 17.
		assert!(goto_definition(main.as_bytes(), &main_path, 2, 17).is_none());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn stdlib_value_resolves_to_baked_module() {
		// `list.reverse` resolves into the baked `core.list` source — no file
		// on disk, no materialization (resolve is pure).
		let main = "use core.list\n\ndef x = list.reverse [1]\n";
		// `reverse` in `list.reverse` is at line 2, col 13.
		match resolve(main.as_bytes(), &PathBuf::from("/proj/main.pa"), 2, 15) {
			Some(Resolved::OtherModule {
				location: ModuleLocation::Stdlib(name),
				module,
				range,
			}) => {
				assert_eq!(name, "core.list");
				// `def reverse` is on line 37 of list.pa (0-indexed 36)... assert
				// it lands on a def whose name is `reverse`.
				let def = module
					.ast
					.as_ref()
					.unwrap()
					.body
					.iter()
					.find(|d| d.name.range.start.line == range.start.line)
					.unwrap();
				assert_eq!(def.name.name, "reverse");
			}
			other => panic!("expected stdlib OtherModule, got {}", resolved_kind(&other)),
		}
	}

	#[test]
	fn stdlib_goto_materializes_to_cache() {
		// Point the cache at a writable temp dir, then jump into a stdlib
		// symbol and confirm the source was written there. The temp dir is
		// injected as a parameter (not via $XDG_CACHE_HOME) so this test
		// mutates no process-global state and can run alongside others.
		let cache = std::env::temp_dir().join(format!("pluma-cache-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&cache);

		let main = "use core.list\n\ndef x = list.reverse [1]\n";
		match goto_definition_in(
			main.as_bytes(),
			&PathBuf::from("/proj/main.pa"),
			2,
			15,
			Some(&cache),
		) {
			Some(Target::OtherFile { path, .. }) => {
				assert!(path.ends_with("core/list.pa"), "path: {:?}", path);
				assert!(path.is_file(), "materialized file should exist");
			}
			other => panic!("expected OtherFile into cache, got {}", target_kind(&other)),
		}
		let _ = std::fs::remove_dir_all(&cache);
	}

	fn target_kind(t: &Option<Target>) -> &'static str {
		match t {
			None => "None",
			Some(Target::Here(_)) => "Here",
			Some(Target::OtherFile { .. }) => "OtherFile",
		}
	}

	fn resolved_kind(t: &Option<Resolved>) -> &'static str {
		match t {
			None => "None",
			Some(Resolved::Here(_)) => "Here",
			Some(Resolved::OtherModule { .. }) => "OtherModule",
		}
	}
}
