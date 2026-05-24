use compiler::ast::*;
use compiler::{Diagnostic, Module, Range};
use std::collections::HashSet;
use std::path::PathBuf;

// Go-to-definition, resolved from parser output alone — the same model as
// `semantic_tokens`. We deliberately skip the full analyzer: it needs the
// project's imports resolved from disk and produces a `!Send` `Module`,
// whereas the syntactic structure is enough to resolve definitions *within
// a single file* (the overwhelming majority of jumps). Cross-module value
// references resolve to the `use` statement that brought the namespace in,
// which still answers "where did this come from?".
//
// Resolution is two halves: collect every binding (with the source region
// it's visible in) and every reference, then for the reference under the
// cursor pick the binding with the matching name whose scope contains the
// cursor, preferring the innermost (smallest) scope so shadowing resolves
// correctly.

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

struct Binding {
	name: String,
	// Where to jump: the identifier's own range.
	def_range: Range,
	// The region the binding is visible in. `None` means module-global
	// (top-level defs, enums, variants, imports) — always in scope.
	scope: Option<Range>,
}

/// Resolve the definition site for whatever identifier sits under the
/// cursor. Returns the definition's range within the same file, or `None`
/// if there's no identifier there or it doesn't resolve locally.
pub fn goto_definition(source: &[u8], line: u32, character: u32) -> Option<Range> {
	let mut module = Module::new("<lsp>".to_string(), PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);
	let ast = module.ast.as_ref()?;

	let mut r = Resolver::new(ast);
	r.walk_module(ast);

	let line = line as usize;
	let character = character as usize;

	// The reference under the cursor: smallest range that contains it.
	let reference = r
		.refs
		.iter()
		.filter(|rf| contains(&rf.range, line, character))
		.min_by_key(|rf| range_size(&rf.range))?;

	let table = match reference.kind {
		RefKind::Value => {
			// Values shadow into variants: a payload-less variant used as a
			// value (`none`) parses as a plain identifier.
			return resolve(&r.values, &reference.name, line, character)
				.or_else(|| resolve(&r.variants, &reference.name, line, character));
		}
		RefKind::Type => &r.types,
		RefKind::Variant => &r.variants,
		RefKind::Namespace => &r.namespaces,
	};
	resolve(table, &reference.name, line, character)
}

fn resolve(table: &[Binding], name: &str, line: usize, character: usize) -> Option<Range> {
	table
		.iter()
		.filter(|b| b.name == name && scope_contains(&b.scope, line, character))
		.min_by_key(|b| scope_size(&b.scope))
		.map(|b| b.def_range)
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
			DefinitionKind::Test { body, .. } => {
				for stmt in body {
					self.walk_expr(stmt, None);
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
						// `module.value`: the namespace resolves to its import;
						// the field lives in another file, so leave it.
						self.reference(id, RefKind::Namespace);
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
			ExprKind::Grouping(inner) => self.walk_expr(inner, scope),
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
						// `module.Type`: the prefix is a namespace; the type
						// itself lives in the other module.
						self.reference(module, RefKind::Namespace);
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

	// Resolve at (line, col) and return the definition as
	// (start_line, start_col) for compact assertions.
	fn goto(src: &str, line: u32, col: u32) -> Option<(usize, usize)> {
		goto_definition(src.as_bytes(), line, col).map(|r| (r.start.line, r.start.col))
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
}
