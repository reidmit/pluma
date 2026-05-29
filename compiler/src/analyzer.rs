use crate::ast::*;
use crate::binding::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::location::Range;
use crate::module::{EnumExport, Module, ModuleExports};
use crate::types::*;
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::PathBuf;
use AnalysisErrorKind::*;

enum VariantResolution {
	Found(String, Vec<Type>),
	NotFound,
	Ambiguous,
}

// Resolved enum definition. `param_vars` are the type-var ids minted when
// the enum was declared (one per declared param); variant params may reference
// them by `Type::Var(id)`. To use the enum at a call site, mint fresh vars
// and substitute the `param_vars` for them — same pattern as `instantiate_with`.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct EnumDef {
	pub param_vars: Vec<usize>,
	pub variants: Vec<(String, Vec<Type>)>,
}

pub struct Analyzer<'compiler> {
	module_name: Option<String>,
	module_path: Option<PathBuf>,
	diagnostics: &'compiler mut Vec<Diagnostic>,
	value_scopes: Vec<HashMap<String, ValueBinding>>,
	type_scope: HashMap<String, TypeBinding>,
	// Enum definitions visible during analysis, keyed by the *qualified*
	// enum name (`<defining-module>.<enum-name>`). Both locally-defined
	// enums and imported ones are stored here under that qualified key so
	// that `Type::Enum(qualified-name, _)` lookups work uniformly.
	enum_defs: HashMap<String, EnumDef>,
	// Reverse map: bare variant name -> list of (qualified-enum, variant-name)
	// pairs. Lets `some 5` (no enum prefix) resolve to its enum's variant
	// constructor, with an `AmbiguousVariant` error if the name appears in
	// more than one enum.
	variant_constructors: HashMap<String, Vec<(String, String)>>,
	// Imports: local namespace name (e.g. `math` from `use math` or `utils`
	// from `use sub.utils`) -> that module's full exports.
	imports: HashMap<String, ModuleExports>,
	// The fully-qualified name of each imported module, keyed by the local
	// namespace name. `use a.b.utils as u` produces `u -> a.b.utils`.
	import_qualified: HashMap<String, String>,
	// One-shot hint: the resolved parameter types of the function about to be
	// constrained as an annotated def's RHS. The `Fun` arm consumes it to seed
	// scope-handle params concretely (so handle methods dispatch on a param).
	// Consumed (taken) by the first `Fun` it reaches, so nested funs don't
	// inherit it.
	fun_param_hints: Option<Vec<Type>>,
	next_type_var_id: usize,
	// Typeclass declarations visible during analysis. Phase 2 seeds
	// `numeric` here directly from the prelude.
	traits: HashMap<String, TraitDecl>,
	// Typeclass instances. Keyed by `(trait_name, head_key)` for fast
	// lookup during discharge. `head_key` is a stable string for the
	// instance's head type (e.g. `"int"`, `"float"`).
	instances: HashMap<(String, String), InstanceDecl>,
	// Fresh class constraints minted during Gen/Inst processing (one set
	// per Inst-against-Gen match). Picked up by `analyze` for discharge.
	fresh_class_constraints: Vec<ClassConstraint>,
	// Per-def class constraints from resolve_forwarded — one entry per
	// dict param of the def's scheme. Used to build cross-module
	// `value_constraints` exports so importing modules can stitch in
	// dict args at call sites.
	def_value_constraints: HashMap<String, Vec<crate::module::ValueConstraintExport>>,
	// Explicit `where (trait param, ...)` constraints declared on a def's
	// signature, keyed by def name. Each entry pairs a trait name with the
	// annotation tyvar its `param` resolved to (pre-substitution). Merged
	// into `def_value_constraints` after solving — this is how a `built-in`
	// body with no dispatch cells still exports a dict-threading contract.
	def_where_clauses: HashMap<String, Vec<(String, usize)>>,
	// Prelude exports passed in by the compiler. Seeded into this
	// analyzer's enum / variant / instance tables during `analyze()` so
	// the user module sees prelude types and instances without needing
	// an explicit `use __prelude__`.
	prelude_exports: Option<ModuleExports>,
}

// Analyzer-side view of a trait declaration. Method types reference the
// trait's `param_var`; each use site instantiates with a fresh var.
// `defaults` holds the AST template for each method that has a `default`
// body — instances missing those methods clone the template into their
// own method list before constraining. (The field is currently only
// populated for diagnostic / introspection use; the actual cloning is
// done from the trait's `TraitNode` AST in `constrain`'s pre-pass.)
#[allow(dead_code)]
pub struct TraitDecl {
	pub param_var: usize,
	pub method_order: Vec<String>,
	pub method_types: HashMap<String, Type>,
	pub defaults: HashMap<String, ExprNode>,
	// Module that declared this trait. Used by the orphan-rule check to
	// reject `for T on U` declared in a module that owns neither T nor U.
	// Prelude traits use `"__prelude__"`.
	pub defining_module: String,
}

// Analyzer-side view of an instance.
//
// Concrete instances have empty `param_vars` and `where_clauses`. The
// `head_type` is a concrete `Type` (e.g. `Type::Int`).
//
// Parametric instances have non-empty `param_vars` and may have
// `where_clauses`. The `head_type` contains those param vars as
// `Type::Var(_)`. Discharge unifies a class constraint's `ty` against
// `head_type`, applies the substitution to the `where_clauses`, and
// recursively discharges them — building an `InstanceChain` for codegen.
#[allow(dead_code)]
pub struct InstanceDecl {
	pub trait_name: String,
	pub head_type: Type,
	pub param_vars: Vec<usize>,
	pub where_clauses: Vec<(String, usize)>,
	pub instance_slot_name: String,
}

// First-seen slot allocation: returns the existing slot index for
// `(trait, var)` in `slot_order`, or appends a new one and returns its
// index. Used by the forwarded-resolution pass to map each dispatch
// tyvar to a stable dict-param slot.
fn lookup_or_alloc_slot(
	slot_order: &mut Vec<(String, usize)>,
	trait_name: &str,
	var: usize,
) -> u16 {
	if let Some(idx) = slot_order
		.iter()
		.position(|(t, v)| t == trait_name && *v == var)
	{
		idx as u16
	} else {
		slot_order.push((trait_name.to_string(), var));
		(slot_order.len() - 1) as u16
	}
}

// One-way type matching: tries to bind each `Type::Var` in `pattern` to
// the corresponding subterm in `target`. Used by discharge to match a
// class constraint's type against a parametric instance's head type.
// Returns `Some(mapping)` on success, `None` on mismatch.
// Substitute the type variables named in `mapping` throughout `ty`, leaving
// everything else intact. Used to specialize an enum variant's declared
// payload types (which reference the enum's param vars) to a concrete
// instantiation when building a `wire` schema.
fn subst_type(ty: &Type, mapping: &HashMap<usize, Type>) -> Type {
	use Type::*;
	match ty {
		Var(v) => mapping.get(v).cloned().unwrap_or_else(|| ty.clone()),
		List(inner) => List(Box::new(subst_type(inner, mapping))),
		Ref(inner) => Ref(Box::new(subst_type(inner, mapping))),
		Tuple(elems) => Tuple(elems.iter().map(|e| subst_type(e, mapping)).collect()),
		Dict(k, v) => Dict(
			Box::new(subst_type(k, mapping)),
			Box::new(subst_type(v, mapping)),
		),
		Enum(name, args) => Enum(
			name.clone(),
			args.iter().map(|a| subst_type(a, mapping)).collect(),
		),
		Record(fields, tail) => Record(
			fields
				.iter()
				.map(|(n, t)| (n.clone(), subst_type(t, mapping)))
				.collect(),
			*tail,
		),
		Fun(params, ret) => Fun(
			params.iter().map(|p| subst_type(p, mapping)).collect(),
			Box::new(subst_type(ret, mapping)),
		),
		_ => ty.clone(),
	}
}

// Dict keys that the `wire` codec can rehash on decode in Rust (so it doesn't
// need the key's `hash` instance). Matches `value::primitive_hash`.
fn is_primitive_wire_key(ty: &Type) -> bool {
	matches!(
		ty,
		Type::Int | Type::Float | Type::Bool | Type::String | Type::Bytes
	)
}

fn match_types(
	pattern: &Type,
	target: &Type,
	mapping: &mut std::collections::HashMap<usize, Type>,
) -> bool {
	use Type::*;
	match (pattern, target) {
		(Var(v), t) => {
			if let Some(existing) = mapping.get(v) {
				type_keys_match(existing, t)
			} else {
				mapping.insert(*v, t.clone());
				true
			}
		}
		(Int, Int)
		| (Float, Float)
		| (Bool, Bool)
		| (String, String)
		| (Regex, Regex)
		| (Instant, Instant)
		| (Duration, Duration)
		| (Nothing, Nothing) => true,
		(Enum(a, args_a), Enum(b, args_b)) if a == b && args_a.len() == args_b.len() => args_a
			.iter()
			.zip(args_b.iter())
			.all(|(p, t)| match_types(p, t, mapping)),
		(List(a), List(b)) => match_types(a, b, mapping),
		(Dict(ka, va), Dict(kb, vb)) => match_types(ka, kb, mapping) && match_types(va, vb, mapping),
		(Ref(a), Ref(b)) => match_types(a, b, mapping),
		(Tuple(a), Tuple(b)) if a.len() == b.len() => a
			.iter()
			.zip(b.iter())
			.all(|(p, t)| match_types(p, t, mapping)),
		(Fun(p_params, p_ret), Fun(t_params, t_ret)) if p_params.len() == t_params.len() => {
			let params_match = p_params
				.iter()
				.zip(t_params.iter())
				.all(|(p, t)| match_types(p, t, mapping));
			params_match && match_types(p_ret, t_ret, mapping)
		}
		_ => false,
	}
}

// Structural equality on types used for class constraint deduplication.
// Only the cases actually used in dispatch (Var, primitives, Enum) — we
// don't currently support parametric instances at the scheme level, so
// other type shapes don't appear.
fn type_keys_match(a: &Type, b: &Type) -> bool {
	match (a, b) {
		(Type::Var(x), Type::Var(y)) => x == y,
		(Type::Int, Type::Int)
		| (Type::Float, Type::Float)
		| (Type::Bool, Type::Bool)
		| (Type::String, Type::String)
		| (Type::Bytes, Type::Bytes)
		| (Type::Regex, Type::Regex)
		| (Type::Instant, Type::Instant)
		| (Type::Duration, Type::Duration)
		| (Type::Nothing, Type::Nothing) => true,
		(Type::Enum(a, _), Type::Enum(b, _)) => a == b,
		_ => false,
	}
}

// Walk an expression tree collecting every dispatch cell it contains
// (both `trait_dispatch` on ExprNodes and `dict_args` on CallNodes).
// Used by the forwarded-resolution pass to scan a def's body.
fn collect_dispatch_cells(expr: &ExprNode, cells: &mut Vec<DispatchCell>) {
	if let Some(cell) = &expr.trait_dispatch {
		cells.push(cell.clone());
	}
	// A constrained value referenced in value position (not a direct callee)
	// keeps its dict cells in an undrained sink — a Call would have drained
	// the callee's sink into its `dict_args` during annotate, which runs
	// before this pass. Those surviving cells still need Forwarded
	// resolution when the reference sits inside a polymorphic def.
	if let Some(sink) = &expr.dispatch_sink {
		for cell in sink.borrow().iter() {
			cells.push(cell.clone());
		}
	}
	match &expr.kind {
		ExprKind::Call(CallNode {
			callee,
			args,
			dict_args,
			..
		}) => {
			collect_dispatch_cells(callee, cells);
			for c in dict_args {
				cells.push(c.clone());
			}
			for a in args {
				collect_dispatch_cells(a, cells);
			}
		}
		ExprKind::Fun(FunNode { body, .. }) => {
			for e in body {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::Let(LetNode { value, .. }) => {
			collect_dispatch_cells(value, cells);
		}
		ExprKind::BinaryOperation { left, right, .. } => {
			collect_dispatch_cells(left, cells);
			collect_dispatch_cells(right, cells);
		}
		ExprKind::UnaryOperation { right, .. } => {
			collect_dispatch_cells(right, cells);
		}
		ExprKind::FieldAccess { receiver, .. } | ExprKind::ElementAccess { receiver, .. } => {
			collect_dispatch_cells(receiver, cells);
		}
		ExprKind::Grouping(inner) => collect_dispatch_cells(inner, cells),
		ExprKind::Defer(inner) => collect_dispatch_cells(inner, cells),
		ExprKind::Tuple(es) | ExprKind::Interpolation(es) => {
			for e in es {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::List(items) => {
			for item in items {
				collect_dispatch_cells(item.expr(), cells);
			}
		}
		ExprKind::Record(fields) => {
			for (_, e) in fields {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::RecordUpdate { base, fields } => {
			collect_dispatch_cells(base, cells);
			for (_, e) in fields {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::If(IfNode {
			subject,
			body,
			else_body,
			..
		}) => {
			collect_dispatch_cells(subject, cells);
			for e in body {
				collect_dispatch_cells(e, cells);
			}
			if let Some(else_body) = else_body {
				for e in else_body {
					collect_dispatch_cells(e, cells);
				}
			}
		}
		ExprKind::While(WhileNode { subject, body, .. }) => {
			collect_dispatch_cells(subject, cells);
			for e in body {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::When(WhenNode { subject, cases, .. }) => {
			collect_dispatch_cells(subject, cells);
			for case in cases {
				for e in &case.body {
					collect_dispatch_cells(e, cells);
				}
			}
		}
		ExprKind::Try(TryNode { value, rest, .. }) => {
			collect_dispatch_cells(value, cells);
			for e in rest {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::Scope(ScopeNode { body, .. }) => {
			for e in body {
				collect_dispatch_cells(e, cells);
			}
		}
		ExprKind::Identifier(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::EmptyTuple
		| ExprKind::Builtin(_)
		| ExprKind::NamespaceAccess(_) => {}
	}
}

// The prelude `scope-handle` type — bound by the fail-fast `scope as s`. A
// variant-less prelude enum (like `task`); the runtime carries a native
// `Value::ScopeHandle`. No type params (its children are heterogeneous).
pub fn scope_handle_type() -> Type {
	Type::Enum("__prelude__.scope-handle".to_string(), vec![])
}

// The prelude `manual-scope-handle a` type — bound by `manual scope as s`.
// Carries the homogeneous child type `a` so `s.next` can return it.
pub fn manual_scope_handle_type(elem: Type) -> Type {
	Type::Enum("__prelude__.manual-scope-handle".to_string(), vec![elem])
}

// Is `t` one of the scope-handle types? Used to seed a function parameter
// concretely from its signature, so `s.spawn`/`s.next` dispatch when a handle
// arrives as a parameter (not just when bound by `scope as s`).
fn is_handle_type(t: &Type) -> bool {
	matches!(
		t,
		Type::Enum(n, _) if n == "__prelude__.scope-handle" || n == "__prelude__.manual-scope-handle"
	)
}

// Which kind of scope handle a binding is. They expose slightly different
// method sets and lower to different (but runtime-identical) kernel defs.
#[derive(Clone, Copy, PartialEq)]
enum HandleKind {
	Scope,
	Manual,
}

// The ML value restriction: a `let` binding may be generalized only when its
// RHS is a *syntactic value* — something that performs no computation and so
// can't observe/capture an effect. Variables, literals, lambdas, namespace/
// builtin references, and aggregates built solely from values qualify;
// function applications (`f x`, `s.spawn t`, `ref.new x`), field/element
// access, operators, and control flow do not.
fn is_syntactic_value(e: &ExprNode) -> bool {
	match &e.kind {
		ExprKind::Fun(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::EmptyTuple
		| ExprKind::Identifier(_)
		| ExprKind::NamespaceAccess(_)
		| ExprKind::Builtin(_) => true,
		ExprKind::Grouping(inner) => is_syntactic_value(inner),
		ExprKind::Tuple(es) => es.iter().all(is_syntactic_value),
		ExprKind::List(items) => items.iter().all(|i| is_syntactic_value(i.expr())),
		ExprKind::Record(fields) => fields.iter().all(|(_, v)| is_syntactic_value(v)),
		_ => false,
	}
}

// Maps a scope-handle method name + handle kind to the `core.task` kernel def
// it lowers to. `None` if the method isn't valid on that handle (e.g. `next`
// on a fail-fast scope), so it falls through to the normal record/error path.
fn scope_method_def(method: &str, kind: HandleKind) -> Option<&'static str> {
	match (kind, method) {
		(HandleKind::Scope, "spawn") => Some("scope-spawn"),
		(HandleKind::Scope, "cancel") => Some("scope-cancel"),
		(HandleKind::Scope, "cancel-after") => Some("scope-cancel-after"),
		(HandleKind::Manual, "spawn") => Some("manual-spawn"),
		(HandleKind::Manual, "cancel") => Some("manual-cancel"),
		(HandleKind::Manual, "cancel-after") => Some("manual-cancel-after"),
		(HandleKind::Manual, "next") => Some("manual-next"),
		_ => None,
	}
}

// The module that owns a type's outer constructor. For primitives and
// prelude-defined enums this is `"__prelude__"`. For user-defined enums
// it's the module prefix of the qualified name. Used by the orphan-rule
// check to decide whether an instance declaration is allowed in the
// current module.
pub fn type_defining_module(ty: &Type) -> Option<String> {
	match ty {
		Type::Int
		| Type::Float
		| Type::Bool
		| Type::String
		| Type::Bytes
		| Type::Regex
		| Type::Instant
		| Type::Duration
		| Type::Nothing => Some("__prelude__".into()),
		Type::Enum(name, _) => Some(
			name
				.rsplit_once('.')
				.map(|(m, _)| m.to_string())
				.unwrap_or_else(|| "__prelude__".into()),
		),
		Type::List(_) => Some("__prelude__".into()),
		Type::Dict(_, _) => Some("__prelude__".into()),
		Type::Ref(_) => Some("__prelude__".into()),
		_ => None,
	}
}

// Stable key for instance lookup. Concrete primitives map to their own
// names; enums use their qualified name. Phase 3 will need to extend
// this for parametric heads, but for phase 2 we only see fully concrete
// dispatch types.
pub fn type_to_head_key(ty: &Type) -> Option<String> {
	match ty {
		Type::Int => Some("int".into()),
		Type::Float => Some("float".into()),
		Type::Bool => Some("bool".into()),
		Type::String => Some("string".into()),
		Type::Bytes => Some("bytes".into()),
		Type::Regex => Some("regex".into()),
		Type::Instant => Some("instant".into()),
		Type::Duration => Some("duration".into()),
		Type::Nothing => Some("nothing".into()),
		Type::Enum(name, _) => Some(name.clone()),
		Type::List(_) => Some("__list__".into()),
		Type::Dict(_, _) => Some("__dict__".into()),
		Type::Ref(_) => Some("__ref__".into()),
		_ => None,
	}
}

impl<'compiler> Analyzer<'compiler> {
	/// Creates a new `Analyzer`. Takes a mutable list of diagnostics
	/// to which any analyis errors/warnings will be appended.
	pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
		Self {
			module_name: None,
			module_path: None,
			diagnostics,
			value_scopes: Vec::new(),
			type_scope: HashMap::new(),
			enum_defs: HashMap::new(),
			variant_constructors: HashMap::new(),
			imports: HashMap::new(),
			import_qualified: HashMap::new(),
			fun_param_hints: None,
			next_type_var_id: 0,
			traits: HashMap::new(),
			instances: HashMap::new(),
			fresh_class_constraints: Vec::new(),
			def_value_constraints: HashMap::new(),
			def_where_clauses: HashMap::new(),
			prelude_exports: None,
		}
	}

	/// Runs analysis over a parsed module. The AST will be annotated
	/// with inferred types (hence the mutability).
	pub fn analyze(&mut self, module: &mut Module) {
		self.module_name = Some(module.module_name.clone());
		self.module_path = Some(module.module_path.clone());

		// TODO: We're adding the builtin types here, but there must be a better way
		self.add_type_binding("int".into(), Type::Int, Range::collapsed(0, 0));
		self.add_type_binding("bool".into(), Type::Bool, Range::collapsed(0, 0));
		self.add_type_binding("string".into(), Type::String, Range::collapsed(0, 0));
		self.add_type_binding("bytes".into(), Type::Bytes, Range::collapsed(0, 0));
		self.add_type_binding("regex".into(), Type::Regex, Range::collapsed(0, 0));
		self.add_type_binding("instant".into(), Type::Instant, Range::collapsed(0, 0));
		self.add_type_binding("duration".into(), Type::Duration, Range::collapsed(0, 0));
		self.add_type_binding("float".into(), Type::Float, Range::collapsed(0, 0));
		self.add_type_binding("nothing".into(), Type::Nothing, Range::collapsed(0, 0));

		// Seed enum_defs with imported enums under their canonical
		// `<defining-module>.<enum-name>` keys, so variant resolution and
		// exhaustiveness checks can see them. Exported variant params reference
		// canonical Var ids `0..param_count-1`; we mint fresh local vars and
		// substitute so the imported enum lives in our own var namespace.
		let imported_enums: Vec<(String, String, EnumExport)> = self
			.imports
			.iter()
			.flat_map(|(local_name, exports)| {
				let qualified_module = self
					.import_qualified
					.get(local_name)
					.cloned()
					.unwrap_or_else(|| local_name.clone());
				exports.enums.iter().map(move |(enum_name, enum_export)| {
					(
						qualified_module.clone(),
						enum_name.clone(),
						enum_export.clone(),
					)
				})
			})
			.collect();
		for (qualified_module, enum_name, enum_export) in imported_enums {
			let qualified = format!("{}.{}", qualified_module, enum_name);
			let fresh_param_vars: Vec<usize> = (0..enum_export.param_count)
				.map(|_| {
					let id = self.next_type_var_id;
					self.next_type_var_id += 1;
					id
				})
				.collect();
			let rebind = Substitution {
				solutions: (0..enum_export.param_count)
					.map(|i| (i, Type::Var(fresh_param_vars[i])))
					.collect(),
				row_solutions: HashMap::new(),
				tuple_row_solutions: HashMap::new(),
			};
			let variants: Vec<(String, Vec<Type>)> = enum_export
				.variants
				.into_iter()
				.map(|(n, params)| {
					let rebound = params
						.into_iter()
						.map(|p| rebind.apply_to_type(&p))
						.collect();
					(n, rebound)
				})
				.collect();
			for (variant_name, _) in &variants {
				self
					.variant_constructors
					.entry(variant_name.clone())
					.or_default()
					.push((qualified.clone(), variant_name.clone()));
			}
			self.enum_defs.insert(
				qualified,
				EnumDef {
					param_vars: fresh_param_vars,
					variants,
				},
			);
		}

		self.enter_scope();

		// Prelude: builtin values visible in every module.
		// `print: forall a. a -> nothing` — write the value to stdout
		// (rendered via the same Display the VM uses for to-string).
		let print_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		self.add_value_binding(
			"print".into(),
			Scheme::Forall(
				vec![print_var],
				vec![],
				vec![],
				Type::Fun(vec![Type::Var(print_var)], Box::new(Type::Nothing)),
			),
			Range::collapsed(0, 0),
		);
		// `debug: forall a. a -> a` — like `print`, but prints a
		// `<module>:<line>` header above the value and returns it unchanged
		// so it can be dropped into a pipeline without breaking it.
		let debug_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		self.add_value_binding(
			"debug".into(),
			Scheme::Forall(
				vec![debug_var],
				vec![],
				vec![],
				Type::Fun(vec![Type::Var(debug_var)], Box::new(Type::Var(debug_var))),
			),
			Range::collapsed(0, 0),
		);
		// `to-string: forall a. a -> string` — render any value as a string.
		// Like `print`, dispatches on the runtime tag — the one function whose
		// polymorphism the type system can't otherwise express. Revisit when
		// generics land.
		let to_string_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		self.add_value_binding(
			"to-string".into(),
			Scheme::Forall(
				vec![to_string_var],
				vec![],
				vec![],
				Type::Fun(vec![Type::Var(to_string_var)], Box::new(Type::String)),
			),
			Range::collapsed(0, 0),
		);

		// Implicit prelude import. Every user module sees `__prelude__`'s
		// enums (option, result, ordering) and their variant
		// constructors. The prelude module itself doesn't get this — it
		// declares those enums in its own body.
		if let Some(prelude) = self.prelude_exports.clone() {
			self.seed_imported_enums("__prelude__", &prelude.enums, true);
		}

		// Prelude trait + instances: `numeric` on `int` and `float`. Seeded
		// directly (skipping the user-facing trait/instance defs) so every
		// module sees the trait + can dispatch on int/float arithmetic
		// from the start.
		self.register_prelude_numeric_trait();
		// `ord` trait: `compare fun (a, a) -> ordering`. Concrete instances
		// on int, float, string. Parametric `ord` on `option a` / `list a`
		// added below once the prelude enum types are registered.
		self.register_prelude_ord_trait();
		// `hash` trait: `hash fun a -> int`. Concrete instances on int,
		// float, string, bool. Unblocks generic `core.dict` over those
		// primitive key types.
		self.register_prelude_hash_trait();
		// `wire` trait: `encode fun a -> bytes` / `decode fun bytes ->
		// result a wire-error`. Auto-derived structurally (FULLSTACK.md): no
		// concrete instances — dispatch is resolved by synthesizing a schema
		// from the type's shape in `try_resolve_dispatch`.
		self.register_prelude_wire_trait();

		// PLUMA_TIMING=2 prints per-phase timing within analyze().
		let _timing = std::env::var("PLUMA_TIMING").ok().as_deref() == Some("2");
		let mut _t_constrain = std::time::Duration::ZERO;
		let mut _t_unify = std::time::Duration::ZERO;
		let mut _t_try = std::time::Duration::ZERO;
		let mut _t_discharge = std::time::Duration::ZERO;
		let mut _t_annotate = std::time::Duration::ZERO;
		let mut _n_constraints = 0usize;

		// the four basic phases of analysis!
		let substitution = if let Some(ast) = &mut module.ast {
			// 1. generate constraints based on AST (and also fill in any
			//    types we can infer without constraints, like for literals)
			let _c0 = std::time::Instant::now();
			let constraints = self.constrain(ast);
			_t_constrain = _c0.elapsed();
			_n_constraints = constraints.len();

			// 2. find a solution that unifies all the constraints. Class
			//    constraints flow with the rest so generalize_with_constraints
			//    sees them; they're collected for discharge afterwards.
			//    Inst-instantiated fresh class constraints get stashed on
			//    the analyzer and merged below.
			let _u0 = std::time::Instant::now();
			let substitution = self.unify(&constraints);
			_t_unify = _u0.elapsed();

			// 2b. type-directed dispatch + rewrite for `try`. Walks the AST,
			//     reads each `try`'s RHS head constructor (substituted), and
			//     rewrites it into a `<carrier>.then` call. May emit
			//     additional linking constraints; if so, re-unify with them
			//     appended so the new tyvars resolve before annotate.
			//
			//     We iterate to a fixed point because dispatching one `try`
			//     can pin the return type of an enclosing def, which then
			//     unlocks dispatch for `try`s in callers. Stuck `try`s
			//     (whose RHS stays a free tyvar) are flagged after the loop
			//     by `report_unresolved_try_nodes`.
			let _tr0 = std::time::Instant::now();
			let mut substitution = substitution;
			let mut accumulated_constraints = constraints.clone();
			loop {
				let mut extra_constraints = Vec::new();
				let dispatched_any = self.dispatch_try_nodes(ast, &substitution, &mut extra_constraints);
				if !dispatched_any {
					break;
				}
				accumulated_constraints.extend(extra_constraints);
				substitution = self.unify(&accumulated_constraints);
			}
			self.report_unresolved_try_nodes(ast, &substitution);
			let constraints = accumulated_constraints;
			_t_try = _tr0.elapsed();

			// 3. gather all class constraints — originals from constrain
			//    plus the fresh ones minted during Gen/Inst processing —
			//    apply the substitution, and run discharge. Discharge
			//    resolves concrete dispatches into `Resolved::Global` and
			//    leaves remaining variables alone.
			let mut class_constraints: Vec<ClassConstraint> = constraints
				.iter()
				.filter_map(|c| match c {
					Constraint::Class(c) => Some(ClassConstraint {
						name: c.name.clone(),
						ty: substitution.apply_to_type(&c.ty),
						reason: c.reason.clone(),
						dispatch_cell: c.dispatch_cell.clone(),
					}),
					_ => None,
				})
				.collect();
			for c in std::mem::take(&mut self.fresh_class_constraints) {
				class_constraints.push(ClassConstraint {
					name: c.name,
					ty: substitution.apply_to_type(&c.ty),
					reason: c.reason,
					dispatch_cell: c.dispatch_cell,
				});
			}
			let _d0 = std::time::Instant::now();
			self.discharge(&class_constraints);
			_t_discharge = _d0.elapsed();

			// 4. apply the solution to the AST, filling in type variables
			//    that we generated in phase 1
			let _a0 = std::time::Instant::now();
			self.annotate(ast, &substitution);
			_t_annotate = _a0.elapsed();

			// 5. resolve Forwarded dispatches per top-level def. After
			//    discharge, cells with concrete dispatch types are set to
			//    Global; cells whose dispatch type is still a Var get
			//    Forwarded(slot) here, where `slot` is the index of the
			//    matching class constraint in the def's scheme.
			self.resolve_forwarded_dispatches(ast, &substitution);

			Some(substitution)
		} else {
			None
		};

		if _timing {
			let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
			eprintln!(
				"  [phases] {:<16} constrain {:>7.2}  unify {:>7.2}  try {:>7.2}  discharge {:>7.2}  annotate {:>7.2}  ({} constraints)",
				module.module_name,
				ms(_t_constrain),
				ms(_t_unify),
				ms(_t_try),
				ms(_t_discharge),
				ms(_t_annotate),
				_n_constraints,
			);
		}

		// Build the module's exports. Values come from the inferred types of
		// each top-level expr def. Aliases are resolved by applying the
		// substitution to the alias's type binding. Enums are pulled from
		// enum_defs by qualified name and re-keyed by bare name.
		let mut exports = ModuleExports::default();
		if let Some(ast) = &module.ast {
			for def in &ast.body {
				match &def.kind {
					DefinitionKind::Expr(expr) => {
						// Value defs are either `public` (exported) or private
						// (the default). `opaque` is rejected on them at parse
						// time, so only these two cases reach here.
						if def.visibility == Visibility::Public {
							exports
								.values
								.insert(def.name.name.clone(), expr.ty.clone());
							if let Some(cs) = self.def_value_constraints.get(&def.name.name) {
								exports
									.value_constraints
									.insert(def.name.name.clone(), cs.clone());
							}
						} else {
							exports.private.insert(def.name.name.clone());
						}
					}
					DefinitionKind::Alias(_) => {
						if def.visibility != Visibility::Public {
							exports.private.insert(def.name.name.clone());
						} else if let Some(binding) = self.type_scope.get(&def.name.name) {
							// Alias types are exported both as types (for use in
							// type positions like `module.alias-name`) and as
							// constructor functions (for use in value positions
							// like `module.alias-name { ... }`).
							let resolved = match &substitution {
								Some(s) => s.apply_to_type(&binding.ty),
								None => binding.ty.clone(),
							};
							exports
								.aliases
								.insert(def.name.name.clone(), resolved.clone());
							exports.values.insert(
								def.name.name.clone(),
								Type::Fun(vec![resolved.clone()], Box::new(resolved)),
							);
						}
					}
					DefinitionKind::Enum(_) => {
						if def.visibility == Visibility::Private {
							exports.private.insert(def.name.name.clone());
						} else {
							let qualified = format!("{}.{}", module.module_name, def.name.name);
							if let Some(enum_def) = self.enum_defs.get(&qualified) {
								// Canonicalize variant params: local fresh vars (e.g.
								// 42, 43) get rewritten to Var(0), Var(1), ... so
								// importers see a stable, var-namespace-independent
								// signature.
								let canonicalize = Substitution {
									solutions: enum_def
										.param_vars
										.iter()
										.enumerate()
										.map(|(i, local)| (*local, Type::Var(i)))
										.collect(),
									row_solutions: HashMap::new(),
									tuple_row_solutions: HashMap::new(),
								};
								// `opaque` exports the type name but withholds its
								// constructors: importers get an empty variant list,
								// so they can name the type yet can't construct or
								// pattern-match its values. `param_count` is still
								// exported so the type takes the right arguments.
								let variants: Vec<(String, Vec<Type>)> = if def.visibility == Visibility::Opaque {
									Vec::new()
								} else {
									enum_def
										.variants
										.iter()
										.map(|(n, params)| {
											let mapped = params
												.iter()
												.map(|p| canonicalize.apply_to_type(p))
												.collect();
											(n.clone(), mapped)
										})
										.collect()
								};
								exports.enums.insert(
									def.name.name.clone(),
									EnumExport {
										param_count: enum_def.param_vars.len(),
										variants,
									},
								);
							}
						}
					}
					// Trait/Instance: not subject to the visibility ladder.
					// Instances are always exported (via the loop below);
					// traits aren't carried through `ModuleExports` at all.
					DefinitionKind::Trait(_) | DefinitionKind::Instance(_) => {}
				}
			}
		}

		// Export every registered instance whose slot lives in this module.
		// Param tyvars get canonicalized to `Var(0..K-1)` so importers can
		// freshen them into their own namespace.
		let module_prefix = format!("{}.", module.module_name);
		for inst in self.instances.values() {
			if !inst.instance_slot_name.starts_with(&module_prefix) {
				continue;
			}
			let param_count = inst.param_vars.len();
			let (head_type, where_clauses) = if param_count == 0 {
				(
					inst.head_type.clone(),
					inst
						.where_clauses
						.iter()
						.map(|(t, v)| (t.clone(), *v))
						.collect(),
				)
			} else {
				let mut subst = Substitution::empty();
				for (i, var) in inst.param_vars.iter().enumerate() {
					subst.solutions.insert(*var, Type::Var(i));
				}
				let head = subst.apply_to_type(&inst.head_type);
				let wcs: Vec<(String, usize)> = inst
					.where_clauses
					.iter()
					.map(|(t, v)| {
						let idx = inst.param_vars.iter().position(|p| p == v).unwrap_or(0);
						(t.clone(), idx)
					})
					.collect();
				(head, wcs)
			};
			exports.instances.push(crate::module::InstanceExport {
				trait_name: inst.trait_name.clone(),
				head_type,
				param_count,
				where_clauses,
				instance_slot_name: inst.instance_slot_name.clone(),
			});
		}

		module.exports = Some(exports);
	}

	pub fn set_imports(
		&mut self,
		imports: HashMap<String, ModuleExports>,
		import_qualified: HashMap<String, String>,
	) {
		self.imports = imports;
		self.import_qualified = import_qualified;
	}

	// Make `__prelude__`'s exports implicitly available in this module.
	// The analyzer seeds enums, variant constructors, and instances
	// from these during `analyze()`. Set by the compiler for every user
	// module; left `None` when analyzing the prelude itself.
	pub fn set_prelude_exports(&mut self, exports: ModuleExports) {
		self.prelude_exports = Some(exports);
	}

	// Seed instances from a list of exports. Used for prelude instances
	// that are implicitly available in every module — the compiler passes
	// `__prelude__`'s `ModuleExports.instances` here. Param tyvars in
	// each export are canonical (0..param_count-1); we mint fresh ids per
	// instance and substitute through `head_type` and `where_clauses`.
	pub fn add_imported_instances(&mut self, exports: &[crate::module::InstanceExport]) {
		for export in exports {
			let head_key = match type_to_head_key(&export.head_type) {
				Some(k) => k,
				None => continue,
			};
			if self
				.instances
				.contains_key(&(export.trait_name.clone(), head_key.clone()))
			{
				// Already seeded (e.g. by `register_prelude_*`) — skip.
				continue;
			}
			// Mint fresh tyvars to replace the canonical 0..param_count-1.
			let fresh: Vec<usize> = (0..export.param_count)
				.map(|_| {
					let id = self.next_type_var_id;
					self.next_type_var_id += 1;
					id
				})
				.collect();
			let head_type = if export.param_count == 0 {
				export.head_type.clone()
			} else {
				let mut subst = Substitution::empty();
				for (i, f) in fresh.iter().enumerate() {
					subst.solutions.insert(i, Type::Var(*f));
				}
				subst.apply_to_type(&export.head_type)
			};
			let where_clauses: Vec<(String, usize)> = export
				.where_clauses
				.iter()
				.map(|(t, idx)| (t.clone(), fresh[*idx]))
				.collect();
			self.instances.insert(
				(export.trait_name.clone(), head_key),
				InstanceDecl {
					trait_name: export.trait_name.clone(),
					head_type,
					param_vars: fresh,
					where_clauses,
					instance_slot_name: export.instance_slot_name.clone(),
				},
			);
		}
	}

	fn diagnostic(&mut self, range: Option<Range>, diag: Diagnostic) {
		let mut diag = diag;

		if let Some(range) = range {
			diag = diag.with_span(range);
		}

		if let Some(module_name) = &self.module_name {
			diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
		}

		self.diagnostics.push(diag)
	}

	fn warning(&mut self, range: Range, kind: AnalysisErrorKind) {
		self.diagnostic(Some(range), Diagnostic::warning(AnalysisError { kind }));
	}

	fn error(&mut self, range: Range, kind: AnalysisErrorKind) {
		self.diagnostic(Some(range), Diagnostic::error(AnalysisError { kind }));
	}

	fn check_regex_character_classes(&mut self, node: &RegexNode) {
		use RegexKind::*;
		match &node.kind {
			CharacterClass(name) => {
				if !is_known_regex_character_class(name) {
					self.error(
						node.range,
						AnalysisErrorKind::UnknownRegexCharacterClass { name: name.clone() },
					);
				}
			}
			Literal(_) | Anchor(_) => {}
			OneOrMore(inner)
			| ZeroOrMore(inner)
			| OneOrZero(inner)
			| AtLeastCount(inner, _)
			| AtMostCount(inner, _)
			| ExactCount(inner, _)
			| RangeCount(inner, _, _)
			| Grouping(inner)
			| NamedCapture(_, inner) => self.check_regex_character_classes(inner),
			Sequence(parts) | Alternation(parts) => {
				for p in parts {
					self.check_regex_character_classes(p);
				}
			}
		}
	}

	fn add_value_binding(&mut self, name: String, ty_scheme: Scheme, range: Range) {
		let current_level = self.value_scopes.last_mut().expect("no current scope");

		current_level.insert(
			name,
			ValueBinding {
				ty_scheme,
				ref_count: 0,
				range,
			},
		);
	}

	fn get_value_binding(&mut self, name: &String) -> Option<&ValueBinding> {
		for level in self.value_scopes.iter_mut().rev() {
			if let Some(binding) = level.get_mut(name) {
				binding.ref_count += 1;

				return Some(binding);
			}
		}

		None
	}

	fn enter_scope(&mut self) {
		self.value_scopes.push(HashMap::new());
	}

	fn leave_scope(&mut self) {
		if let Some(exited_level) = self.value_scopes.pop() {
			for (name, binding) in exited_level {
				if binding.ref_count == 0 && !name.starts_with("_") {
					self.warning(binding.range, UnusedBinding { name });
				}
			}
		}
	}

	fn add_type_binding(&mut self, name: String, ty: Type, range: Range) {
		self.type_scope.insert(
			name,
			TypeBinding {
				ty,
				ref_count: 0,
				_range: range,
			},
		);
	}

	fn get_type_binding(&mut self, name: &String) -> Option<&TypeBinding> {
		if let Some(binding) = self.type_scope.get_mut(name) {
			binding.ref_count += 1;

			return Some(binding);
		}

		None
	}

	fn constrain(&mut self, module: &mut ModuleNode) -> Vec<Constraint> {
		let mut constraints = Vec::new();
		let mut schemes = Vec::new();
		let mut type_def_vars = Vec::new();
		// Per-enum-def: the type-var ids minted for its declared params.
		// Set during the first pass and consumed in the second pass when we
		// resolve variant param types (the params need the vars in scope so
		// references like `some a` map to the right var).
		let mut enum_param_vars: HashMap<String, Vec<usize>> = HashMap::new();

		// Pre-pass: fill in default methods on instances. For each trait,
		// collect a map of `method_name → default ExprNode`. Then for each
		// instance, for every method present in the trait's defaults but
		// not in the instance, clone the default into the instance's
		// methods list. This keeps the rest of analysis trait-aware in only
		// one place (the trait registration step) — once filled in,
		// instance methods look like ordinary user-written methods.
		let mut trait_defaults: HashMap<String, HashMap<String, ExprNode>> = HashMap::new();
		for def in &module.body {
			if let DefinitionKind::Trait(trait_node) = &def.kind {
				let mut defaults = HashMap::new();
				for m in &trait_node.methods {
					if let Some(default_expr) = &m.default {
						defaults.insert(m.name.name.clone(), default_expr.clone());
					}
				}
				if !defaults.is_empty() {
					trait_defaults.insert(def.name.name.clone(), defaults);
				}
			}
		}
		for def in &mut module.body {
			if let DefinitionKind::Instance(instance_node) = &mut def.kind {
				let defaults = match trait_defaults.get(&instance_node.trait_name.name) {
					Some(d) => d,
					None => continue,
				};
				let present: std::collections::HashSet<String> = instance_node
					.methods
					.iter()
					.map(|m| m.name.name.clone())
					.collect();
				for (method_name, default_expr) in defaults {
					if !present.contains(method_name) {
						instance_node.methods.push(DefinitionNode {
							name: IdentifierNode {
								range: instance_node.range,
								name: method_name.clone(),
							},
							range: instance_node.range,
							kind: DefinitionKind::Expr(default_expr.clone()),
							visibility: Visibility::Private,
							ty: Type::Unknown,
							dict_param_count: 0,
							type_annotation: None,
							where_clause: Vec::new(),
						});
					}
				}
			}
		}

		// first, do a shallow pass to annotate all top-level defs and add them to the scope,
		// so that they can be referenced anywhere within the bodies of other defs
		let mut seen_names: HashMap<String, Range> = HashMap::new();
		for definition in &mut module.body {
			definition.ty = self.new_type_var();

			// Top-level redefinition is an error. Locals can shadow via let,
			// but two `def`s with the same name at module top level is almost
			// certainly a mistake.
			if let Some(_prev_range) =
				seen_names.insert(definition.name.name.clone(), definition.name.range)
			{
				self.error(
					definition.name.range,
					DuplicateDefinition {
						name: definition.name.name.clone(),
					},
				);
			}

			match &mut definition.kind {
				DefinitionKind::Expr(_) => {
					// Similar to lets, we generate a new type scheme for the definition body.
					// This allows defs to be polymorphic (e.g. `def identity fun x { x }`) -
					// these can be instantiated later into concrete types when used.
					let type_scheme = self.new_type_scheme_var();

					self.add_value_binding(
						definition.name.name.clone(),
						type_scheme.clone(),
						definition.name.range,
					);

					schemes.push(type_scheme);
				}

				DefinitionKind::Alias(_) => {
					// Add a type binding for the type defined here...
					let type_var = self.new_type_var();
					self.add_type_binding(
						definition.name.name.clone(),
						type_var.clone(),
						definition.name.range,
					);
					type_def_vars.push(type_var);

					// And also a value binding for the constructor function!
					let type_scheme = self.new_type_scheme_var();
					self.add_value_binding(
						definition.name.name.clone(),
						type_scheme.clone(),
						definition.name.range,
					);
					schemes.push(type_scheme);
				}

				DefinitionKind::Enum(enum_node) => {
					// Enums are nominal: bind the enum type directly. No value binding —
					// the bare name isn't a value, it's only used as a namespace for
					// variant access (e.g. `color.red`), which is resolved via enum_defs.
					// The canonical type name is qualified with the defining module so
					// same-named enums from different modules don't unify.
					let qualified = format!(
						"{}.{}",
						self.module_name.as_ref().unwrap(),
						definition.name.name
					);

					// Mint one fresh type var per declared param. The binding's
					// type carries these vars as its args — a template that any
					// future use of the bare name (in a type position) will
					// substitute against.
					let param_var_ids: Vec<usize> = (0..enum_node.params.len())
						.map(|_| {
							let id = self.next_type_var_id;
							self.next_type_var_id += 1;
							id
						})
						.collect();
					let param_var_types: Vec<Type> = param_var_ids.iter().map(|id| Type::Var(*id)).collect();

					self.add_type_binding(
						definition.name.name.clone(),
						Type::Enum(qualified.clone(), param_var_types),
						definition.name.range,
					);
					enum_param_vars.insert(qualified, param_var_ids);
				}

				DefinitionKind::Trait(trait_node) => {
					// Mint a fresh tyvar for the trait's param (`a`), bind it
					// in the type scope so method signatures can reference it,
					// resolve each signature to a concrete `Type`, and register
					// the trait in `self.traits`. Method types reference the
					// param tyvar via `Type::Var(param_var)`, the same shape
					// the prelude `numeric` trait uses.
					let param_var = self.next_type_var_id;
					self.next_type_var_id += 1;

					let prev = self.type_scope.insert(
						trait_node.param.name.clone(),
						TypeBinding {
							ty: Type::Var(param_var),
							ref_count: 0,
							_range: trait_node.param.range,
						},
					);

					let mut method_order = Vec::new();
					let mut method_types = HashMap::new();
					let mut defaults: HashMap<String, ExprNode> = HashMap::new();
					for m in &trait_node.methods {
						let ty = self.type_expr_to_type(&m.signature, &mut constraints);
						method_order.push(m.name.name.clone());
						method_types.insert(m.name.name.clone(), ty);
						if let Some(default_expr) = &m.default {
							defaults.insert(m.name.name.clone(), default_expr.clone());
						}
					}

					match prev {
						Some(b) => {
							self.type_scope.insert(trait_node.param.name.clone(), b);
						}
						None => {
							self.type_scope.remove(&trait_node.param.name);
						}
					}

					self.traits.insert(
						definition.name.name.clone(),
						TraitDecl {
							param_var,
							method_order,
							method_types,
							defaults,
							defining_module: self.module_name.clone().unwrap_or_default(),
						},
					);
				}

				DefinitionKind::Instance(instance_node) => {
					// Collect parametric type-param names from the where
					// clause. Each `where (trait_name param)` entry names a
					// type var that's bound in both the head and the methods.
					// Parametric instances also exist *without* a where
					// clause (e.g. `for noop on (option a) { ... }`); we
					// detect those by scanning the head for identifiers not
					// in the current type scope (rare; mostly users use
					// `where`).
					let mut param_names: Vec<String> = Vec::new();
					for c in &instance_node.where_clause {
						if !param_names.contains(&c.param.name) {
							param_names.push(c.param.name.clone());
						}
					}

					// Bind each param name to a fresh tyvar in the type scope
					// while we resolve the head + where clauses. Save
					// previous bindings to restore afterwards.
					let mut saved: Vec<(String, Option<TypeBinding>)> = Vec::new();
					let mut param_vars: Vec<usize> = Vec::new();
					for name in &param_names {
						let var = self.next_type_var_id;
						self.next_type_var_id += 1;
						param_vars.push(var);
						let prev = self.type_scope.insert(
							name.clone(),
							TypeBinding {
								ty: Type::Var(var),
								ref_count: 0,
								_range: instance_node.head.range,
							},
						);
						saved.push((name.clone(), prev));
					}

					let head_ty = self.type_expr_to_type(&instance_node.head, &mut constraints);
					let head_key = match type_to_head_key(&head_ty) {
						Some(k) => k,
						None => {
							self.error(
								instance_node.head.range,
								AnalysisErrorKind::UnsupportedInstanceHead {
									head: head_ty.clone(),
								},
							);
							// Restore scope before continuing.
							for (n, prev) in saved {
								match prev {
									Some(b) => {
										self.type_scope.insert(n, b);
									}
									None => {
										self.type_scope.remove(&n);
									}
								}
							}
							continue;
						}
					};

					// Resolve the where clauses to (trait_name, tyvar) pairs.
					let where_clauses: Vec<(String, usize)> = instance_node
						.where_clause
						.iter()
						.filter_map(|c| {
							let idx = param_names.iter().position(|n| n == &c.param.name)?;
							Some((c.trait_name.name.clone(), param_vars[idx]))
						})
						.collect();

					// Restore the type scope — the param names should only be
					// visible inside the instance.
					for (n, prev) in saved {
						match prev {
							Some(b) => {
								self.type_scope.insert(n, b);
							}
							None => {
								self.type_scope.remove(&n);
							}
						}
					}

					let trait_name = instance_node.trait_name.name.clone();
					let module = self.module_name.clone().unwrap_or_default();
					let slot_name = format!("{}.{}@{}", module, trait_name, head_key);
					instance_node.instance_slot_name = slot_name.clone();

					// Orphan rule: the instance must live in the module that
					// declared either the trait or the head type's outer
					// constructor. Without this, two modules could declare
					// conflicting instances on a third module's type.
					let trait_module = self
						.traits
						.get(&trait_name)
						.map(|t| t.defining_module.clone());
					let head_module = type_defining_module(&head_ty);
					let orphan_ok =
						trait_module.as_deref() == Some(&module) || head_module.as_deref() == Some(&module);
					if !orphan_ok && trait_module.is_some() {
						self.error(
							instance_node.range,
							AnalysisErrorKind::OrphanInstance {
								trait_name: trait_name.clone(),
								head: head_ty.clone(),
							},
						);
					}

					let canonical_method_order = self
						.traits
						.get(&trait_name)
						.map(|t| t.method_order.clone())
						.unwrap_or_default();

					// Completeness check: every method the trait declares
					// must either be provided by the instance or have been
					// filled in by the default-method pre-pass.
					let provided: std::collections::HashSet<String> = instance_node
						.methods
						.iter()
						.map(|m| m.name.name.clone())
						.collect();
					for expected in &canonical_method_order {
						if !provided.contains(expected) {
							self.error(
								instance_node.range,
								AnalysisErrorKind::IncompleteInstance {
									trait_name: trait_name.clone(),
									method: expected.clone(),
								},
							);
						}
					}

					instance_node.canonical_method_order = canonical_method_order;

					// Overlap check: refuse to register a second instance with
					// the same (trait, head_key). The hashmap key uses the
					// outer type constructor name, so this catches both
					// `for T on int` + `for T on int` and `for T on (option
					// a)` + `for T on (option b)` (both keyed `option`).
					if self
						.instances
						.contains_key(&(trait_name.clone(), head_key.clone()))
					{
						self.error(
							instance_node.range,
							AnalysisErrorKind::OverlappingInstance {
								trait_name: trait_name.clone(),
								head: head_ty.clone(),
							},
						);
					} else {
						self.instances.insert(
							(trait_name.clone(), head_key),
							InstanceDecl {
								trait_name,
								head_type: head_ty,
								param_vars,
								where_clauses,
								instance_slot_name: slot_name,
							},
						);
					}
				}
			}
		}

		// then, we go through and generate constraints from the defs
		let mut scheme_index = 0;
		let mut type_def_index = 0;

		for definition in &mut module.body {
			// Top-level type annotation, if any. Resolved before constraining
			// the body so the annotation contributes to the constraint set
			// alongside the body's inferred type. The Gen step generalizes
			// over the free type vars introduced by the annotation.
			let (annotated_ty, annotation_vars) = match &definition.type_annotation {
				Some(annotation) => {
					let (ty, vars) = self.resolve_annotation(annotation, &mut constraints);
					(Some(ty), vars)
				}
				None => (None, HashMap::new()),
			};

			// Explicit `where (trait param, ...)` constraints on the
			// signature. Bind each `param` to the tyvar the annotation
			// introduced for it and stash the pair; after solving, these
			// become `value_constraints` exports (see
			// `resolve_forwarded_dispatches`). A `param` that names no free
			// type variable of the annotation is an error.
			if !definition.where_clause.is_empty() {
				let mut recorded: Vec<(String, usize)> = Vec::new();
				for constraint in &definition.where_clause {
					match annotation_vars.get(&constraint.param.name) {
						Some(&var_id) => recorded.push((constraint.trait_name.name.clone(), var_id)),
						None => self.error(
							constraint.param.range,
							WhereClauseParamNotInSignature {
								param: constraint.param.name.clone(),
							},
						),
					}
				}
				if !recorded.is_empty() {
					// For a `built-in`-bodied def, the where-clause is the only
					// source of its class constraints — there's no body whose
					// trait-method usage would generate dispatch cells. Push a
					// synthetic Class constraint per entry so the def's
					// generalized scheme records them; that's what lets a
					// *same-module* caller thread the dicts (cross-module callers
					// read the exported `value_constraints`). Pluma-bodied defs
					// already get these from their body, so we skip them to
					// avoid duplicating dict params.
					let is_builtin = matches!(
						&definition.kind,
						DefinitionKind::Expr(e) if matches!(e.kind, ExprKind::Builtin(_))
					);
					if is_builtin {
						for (trait_name, var_id) in &recorded {
							let ty = Type::Var(*var_id);
							let cell = crate::ast::new_dispatch(trait_name.clone(), None, ty.clone());
							constraints.push(Constraint::Class(ClassConstraint {
								name: trait_name.clone(),
								ty,
								reason: ConstraintReason {
									range: definition.name.range,
								},
								dispatch_cell: cell,
							}));
						}
					}
					self
						.def_where_clauses
						.insert(definition.name.name.clone(), recorded);
				}
			}

			match &mut definition.kind {
				DefinitionKind::Expr(expr) => {
					// `built-in "tag"` as the immediate def RHS: the
					// annotation is the contract; skip the general
					// constrain_expr path so we don't emit a
					// `BuiltinMustBeTopLevelRhs` diagnostic for this
					// legitimate use.
					if matches!(expr.kind, ExprKind::Builtin(_)) {
						match &annotated_ty {
							Some(ty) => {
								expr.ty = ty.clone();
							}
							None => {
								self.error(expr.range, BuiltinRequiresAnnotation);
								expr.ty = self.new_type_var();
							}
						}
					} else {
						// If the signature names scope-handle parameters, hint their
						// types into the `Fun` arm so the handle is concretely typed at
						// constraint time — that's what lets `s.spawn`/`s.next` dispatch
						// on a handle passed *as a parameter*, not just one bound by
						// `scope as s`. (See `maybe_rewrite_scope_method`.)
						if let (Some(Type::Fun(params, _)), ExprKind::Fun(_)) = (&annotated_ty, &expr.kind) {
							if params.iter().any(is_handle_type) {
								self.fun_param_hints = Some(params.clone());
							}
						}
						self.constrain_expr(expr, &mut constraints);
						self.fun_param_hints = None;

						if let Some(annotated_ty) = annotated_ty {
							constraints.push(eq_constraint(expr.ty.clone(), annotated_ty));
						}
					}

					let scheme = schemes.get(scheme_index).unwrap().clone();
					constraints.push(Constraint::Gen(scheme, expr.ty.clone()));
					scheme_index += 1;
				}

				DefinitionKind::Alias(type_expr) => {
					let ty = self.type_expr_to_type(type_expr, &mut constraints);
					let type_var = type_def_vars.get(type_def_index).unwrap().clone();
					constraints.push(eq_constraint(type_var.clone(), ty.clone()));
					type_def_index += 1;

					let scheme = schemes.get(scheme_index).unwrap().clone();
					let constructor_type = Type::Fun(vec![ty.clone()], type_var.clone().into());
					constraints.push(Constraint::Gen(scheme, constructor_type));
					scheme_index += 1;
				}

				DefinitionKind::Enum(enum_node) => {
					let qualified = format!(
						"{}.{}",
						self.module_name.as_ref().unwrap(),
						definition.name.name
					);
					let param_var_ids = enum_param_vars.remove(&qualified).unwrap_or_default();

					// Push each declared param name into the type scope as a
					// `Type::Var` so variant param positions like `some a`
					// resolve to the right var. Saved bindings are restored
					// after.
					let saved: Vec<(String, Option<TypeBinding>)> = enum_node
						.params
						.iter()
						.zip(param_var_ids.iter())
						.map(|(param_ident, var_id)| {
							let prev = self.type_scope.insert(
								param_ident.name.clone(),
								TypeBinding {
									ty: Type::Var(*var_id),
									ref_count: 0,
									_range: param_ident.range,
								},
							);
							(param_ident.name.clone(), prev)
						})
						.collect();

					let variants: Vec<(String, Vec<Type>)> = enum_node
						.variants
						.iter()
						.map(|variant| {
							let params = variant
								.params
								.as_ref()
								.map(|params| {
									params
										.iter()
										.map(|p| self.type_expr_to_type(p, &mut constraints))
										.collect()
								})
								.unwrap_or_default();
							(variant.name.name.clone(), params)
						})
						.collect();

					for (name, prev) in saved {
						match prev {
							Some(prev) => {
								self.type_scope.insert(name, prev);
							}
							None => {
								self.type_scope.remove(&name);
							}
						}
					}

					for (variant_name, _) in &variants {
						self
							.variant_constructors
							.entry(variant_name.clone())
							.or_default()
							.push((qualified.clone(), variant_name.clone()));
					}
					self.enum_defs.insert(
						qualified,
						EnumDef {
							param_vars: param_var_ids,
							variants,
						},
					);
				}

				// Trait declarations are processed in pass 1 (above) — no
				// body-level constraint generation needed here.
				DefinitionKind::Trait(_) => {}

				DefinitionKind::Instance(instance_node) => {
					// Constrain each method body and verify its type matches
					// the trait's expected method type with the trait param
					// substituted by the instance's head type. For parametric
					// instances, bind each `where`-clause tyvar by name so
					// `(option a)` resolves to the same `Type::Var` we
					// allocated in pass 1.
					let trait_name = instance_node.trait_name.name.clone();
					let inst_decl = self.instances.get(&(
						trait_name.clone(),
						instance_node
							.instance_slot_name
							.rsplit_once('@')
							.map(|(_, h)| h.to_string())
							.unwrap_or_default(),
					));
					let (instance_param_vars, instance_param_names): (Vec<usize>, Vec<String>) =
						match inst_decl {
							Some(d) => {
								// Pair each `where` clause's `param` identifier with
								// the matching tyvar; if there are param vars but
								// no where clauses (rare), bind by position.
								let names: Vec<String> = if !instance_node.where_clause.is_empty() {
									instance_node
										.where_clause
										.iter()
										.map(|c| c.param.name.clone())
										.collect()
								} else {
									Vec::new()
								};
								(d.param_vars.clone(), names)
							}
							None => (Vec::new(), Vec::new()),
						};

					let mut saved: Vec<(String, Option<TypeBinding>)> = Vec::new();
					for (name, var) in instance_param_names.iter().zip(instance_param_vars.iter()) {
						let prev = self.type_scope.insert(
							name.clone(),
							TypeBinding {
								ty: Type::Var(*var),
								ref_count: 0,
								_range: instance_node.head.range,
							},
						);
						saved.push((name.clone(), prev));
					}

					let head_ty = self.type_expr_to_type(&instance_node.head, &mut constraints);

					let (param_var, method_types): (usize, HashMap<String, Type>) =
						match self.traits.get(&trait_name) {
							Some(t) => (t.param_var, t.method_types.clone()),
							None => {
								self.error(
									instance_node.trait_name.range,
									AnalysisErrorKind::NameNotBound { name: trait_name },
								);
								// Restore scope before continuing.
								for (n, prev) in saved {
									match prev {
										Some(b) => {
											self.type_scope.insert(n, b);
										}
										None => {
											self.type_scope.remove(&n);
										}
									}
								}
								continue;
							}
						};

					for method in &mut instance_node.methods {
						let expected = match method_types.get(&method.name.name) {
							Some(t) => t,
							None => {
								self.error(
									method.name.range,
									AnalysisErrorKind::NameNotBound {
										name: format!("{}.{}", instance_node.trait_name.name, method.name.name),
									},
								);
								continue;
							}
						};

						// Substitute the trait param tyvar with the head type
						// in the expected method signature, then unify against
						// the method body's inferred type.
						let mut sub = Substitution::empty();
						sub.solutions.insert(param_var, head_ty.clone());
						let expected_substituted = sub.apply_to_type(expected);

						if let DefinitionKind::Expr(expr) = &mut method.kind {
							self.constrain_expr(expr, &mut constraints);
							constraints
								.push(eq_constraint(expr.ty.clone(), expected_substituted).at(method.range));
						}
					}

					// Restore the type scope — param names should only be
					// visible inside the instance body.
					for (n, prev) in saved {
						match prev {
							Some(b) => {
								self.type_scope.insert(n, b);
							}
							None => {
								self.type_scope.remove(&n);
							}
						}
					}
				}
			}
		}

		constraints
	}

	// Build the type-arg vector for an enum reference in a type position.
	// User-provided generics are resolved positionally; missing trailing
	// args are filled with fresh type vars so a bare `option` parses as a
	// polymorphic enum that inference can pin down. Excess generics are an
	// arity-mismatch error.
	fn resolve_enum_args(
		&mut self,
		type_ident: &TypeIdentifierNode,
		expected: usize,
		constraints: &mut Vec<Constraint>,
	) -> Vec<Type> {
		let provided = type_ident.generics.len();
		if provided > expected {
			self.error(
				type_ident.range,
				ParamCountMismatch {
					expected,
					found: provided,
				},
			);
			return vec![Type::Unknown; expected];
		}
		let mut args = Vec::with_capacity(expected);
		for i in 0..expected {
			if i < provided {
				args.push(self.type_expr_to_type(&type_ident.generics[i], constraints));
			} else {
				args.push(self.new_type_var());
			}
		}
		args
	}

	// Walk a type expression and collect identifiers that aren't already
	// bound in the type scope and aren't builtin type names — those are
	// the free type-variable names in a polymorphic annotation like
	// `fun (list a) -> a`. Order is preserved; duplicates are skipped.
	fn collect_free_type_idents(&self, type_expr: &TypeExprNode, out: &mut Vec<String>) {
		match &type_expr.kind {
			TypeExprKind::EmptyTuple => {}
			TypeExprKind::Grouping(inner) => self.collect_free_type_idents(inner, out),
			TypeExprKind::Tuple(entries) => {
				for e in entries {
					self.collect_free_type_idents(e, out);
				}
			}
			TypeExprKind::Record(fields) => {
				for (_, f) in fields {
					self.collect_free_type_idents(f, out);
				}
			}
			TypeExprKind::Func(params, ret) => {
				for p in params {
					self.collect_free_type_idents(p, out);
				}
				self.collect_free_type_idents(ret, out);
			}
			TypeExprKind::Single(type_ident) => {
				let is_builtin = matches!(
					type_ident.name.as_str(),
					"string"
						| "bytes"
						| "int"
						| "float"
						| "bool"
						| "regex"
						| "instant"
						| "duration"
						| "nothing"
						| "list"
						| "dict"
						| "ref"
				);
				if type_ident.module.is_none()
					&& !is_builtin
					&& !self.type_scope.contains_key(&type_ident.name)
					&& !out.contains(&type_ident.name)
				{
					out.push(type_ident.name.clone());
				}
				for g in &type_ident.generics {
					self.collect_free_type_idents(g, out);
				}
			}
		}
	}

	// Resolve a type annotation in a way that lets unbound identifiers act
	// as polymorphic type variables. Mints a fresh type var per free name,
	// inserts it into the type scope, resolves the annotation, then restores
	// the previous bindings. Returns the resolved type plus the free-name →
	// minted-var-id map, so a `where` clause can bind its params to the same
	// tyvars the signature introduced.
	fn resolve_annotation(
		&mut self,
		type_expr: &TypeExprNode,
		constraints: &mut Vec<Constraint>,
	) -> (Type, HashMap<String, usize>) {
		let mut free_names = Vec::new();
		self.collect_free_type_idents(type_expr, &mut free_names);

		let mut saved: Vec<(String, Option<TypeBinding>)> = Vec::with_capacity(free_names.len());
		let mut var_ids: HashMap<String, usize> = HashMap::with_capacity(free_names.len());
		for name in &free_names {
			let var = self.new_type_var();
			if let Type::Var(id) = var {
				var_ids.insert(name.clone(), id);
			}
			let prev = self.type_scope.insert(
				name.clone(),
				TypeBinding {
					ty: var,
					ref_count: 0,
					_range: type_expr.range,
				},
			);
			saved.push((name.clone(), prev));
		}

		let ty = self.type_expr_to_type(type_expr, constraints);

		for (name, prev) in saved {
			match prev {
				Some(prev) => {
					self.type_scope.insert(name, prev);
				}
				None => {
					self.type_scope.remove(&name);
				}
			}
		}

		(ty, var_ids)
	}

	fn type_expr_to_type(
		&mut self,
		type_expr: &TypeExprNode,
		constraints: &mut Vec<Constraint>,
	) -> Type {
		match &type_expr.kind {
			TypeExprKind::EmptyTuple => Type::Nothing,
			TypeExprKind::Grouping(inner) => self.type_expr_to_type(inner, constraints),
			TypeExprKind::Tuple(entries) => Type::Tuple(
				entries
					.into_iter()
					.map(|e| self.type_expr_to_type(e, constraints))
					.collect(),
			),
			TypeExprKind::Record(fields) => Type::Record(
				fields
					.into_iter()
					.map(|(name, f)| (name.name.clone(), self.type_expr_to_type(f, constraints)))
					.collect(),
				None,
			),
			TypeExprKind::Func(params, ret) => Type::Fun(
				params
					.into_iter()
					.map(|p| self.type_expr_to_type(p, constraints))
					.collect(),
				self.type_expr_to_type(ret, constraints).into(),
			),
			TypeExprKind::Single(type_ident) => {
				// `module.TypeName`: look up the type in the named import.
				if let Some(module) = &type_ident.module {
					if let Some(exports) = self.imports.get(&module.name).cloned() {
						if exports.enums.contains_key(&type_ident.name) {
							let qualified_module = self
								.import_qualified
								.get(&module.name)
								.cloned()
								.unwrap_or_else(|| module.name.clone());
							let qualified = format!("{}.{}", qualified_module, type_ident.name);
							// Cross-module generic enums: param count comes from
							// the imported `enum_defs` (populated during import).
							let expected = self
								.enum_defs
								.get(&qualified)
								.map(|d| d.param_vars.len())
								.unwrap_or(0);
							let args = self.resolve_enum_args(type_ident, expected, constraints);
							return Type::Enum(qualified, args);
						}

						if let Some(alias_ty) = exports.aliases.get(&type_ident.name) {
							return alias_ty.clone();
						}

						if exports.private.contains(&type_ident.name) {
							self.error(
								type_ident.range,
								ItemPrivate {
									name: type_ident.name.clone(),
									module: module.name.clone(),
								},
							);
						} else {
							self.error(
								type_ident.range,
								NameNotBound {
									name: format!("{}.{}", module.name, type_ident.name),
								},
							);
						}
						return Type::Unknown;
					}

					self.error(
						module.range,
						NameNotBound {
							name: module.name.clone(),
						},
					);
					return Type::Unknown;
				}

				match &type_ident.name[..] {
					"string" => return Type::String,
					"bytes" => return Type::Bytes,
					"int" => return Type::Int,
					"float" => return Type::Float,
					"bool" => return Type::Bool,
					"regex" => return Type::Regex,
					"instant" => return Type::Instant,
					"duration" => return Type::Duration,
					"nothing" => return Type::Nothing,
					"list" => {
						// `list a` — one type parameter; missing args become fresh vars
						// so a bare `list` parses as a polymorphic list.
						let args = self.resolve_enum_args(type_ident, 1, constraints);
						return Type::List(Box::new(args.into_iter().next().unwrap()));
					}
					"dict" => {
						let args = self.resolve_enum_args(type_ident, 2, constraints);
						let mut iter = args.into_iter();
						let k = iter.next().unwrap();
						let v = iter.next().unwrap();
						return Type::Dict(Box::new(k), Box::new(v));
					}
					"ref" => {
						let args = self.resolve_enum_args(type_ident, 1, constraints);
						return Type::Ref(Box::new(args.into_iter().next().unwrap()));
					}
					_ => {
						if let Some(binding) = self.get_type_binding(&type_ident.name) {
							// For generic enums, the binding holds a template like
							// `Type::Enum(qualified, [Var(p_0), Var(p_1)])`. Each
							// use site builds its own enum type with user-provided
							// generics (or fresh vars if none), so different uses
							// don't accidentally unify through the shared template.
							let binding_ty = binding.ty.clone();
							if let Type::Enum(qualified, template_args) = binding_ty {
								let expected = template_args.len();
								let args = self.resolve_enum_args(type_ident, expected, constraints);
								return Type::Enum(qualified, args);
							}
							return binding_ty;
						}
					}
				}

				self.error(
					type_ident.range,
					NameNotBound {
						name: type_ident.name.clone(),
					},
				);

				Type::Unknown
			}
		}
	}

	// If `name` is bound (in the current scopes) to a scope handle — a
	// monomorphic `scope-handle` or `manual-scope-handle a` — returns which kind.
	// `None` otherwise. (How `scope as NAME` / `manual scope as NAME` bind.)
	fn handle_kind_of_binding(&mut self, name: &String) -> Option<HandleKind> {
		match self.get_value_binding(name) {
			Some(ValueBinding {
				ty_scheme: Scheme::Forall(vars, rows, classes, Type::Enum(n, _)),
				..
			}) if vars.is_empty() && rows.is_empty() && classes.is_empty() => match n.as_str() {
				"__prelude__.scope-handle" => Some(HandleKind::Scope),
				"__prelude__.manual-scope-handle" => Some(HandleKind::Manual),
				_ => None,
			},
			_ => None,
		}
	}

	// The structured-concurrency kernel defs (`scope-spawn`, `manual-next`, …)
	// are private to `core.task`: they're the lowering targets of the `scope`
	// keyword and the `s.spawn`/`s.next` handle methods, not a public API, so
	// their signatures aren't carried in `core.task`'s exports. But a handle
	// method rewritten to `task.scope-spawn …` still needs to type-check, so
	// synthesize their (fixed, compiler-known) signatures here — with fresh
	// tyvars for per-use polymorphism, mirroring how an imported value would be
	// instantiated. `None` for anything that isn't one of these kernel defs.
	// Codegen resolves the call via `lookup_global`, which ignores visibility,
	// so nothing downstream needs the def to be public. Keep these in lockstep
	// with the signatures in `compiler/src/stdlib/task.pa`.
	fn scope_kernel_def_type(&mut self, module: &str, def_name: &str) -> Option<Type> {
		if module != "core.task" {
			return None;
		}
		let task = |a: Type| Type::Enum("__prelude__.task".to_string(), vec![a]);
		match def_name {
			"scope-spawn" => {
				let a = self.new_type_var();
				Some(Type::Fun(
					vec![scope_handle_type(), task(a.clone())],
					Box::new(task(a)),
				))
			}
			"scope-cancel" => Some(Type::Fun(
				vec![scope_handle_type(), Type::Nothing],
				Box::new(Type::Nothing),
			)),
			"scope-cancel-after" => Some(Type::Fun(
				vec![scope_handle_type(), Type::Duration],
				Box::new(Type::Nothing),
			)),
			"manual-spawn" => {
				let a = self.new_type_var();
				Some(Type::Fun(
					vec![manual_scope_handle_type(a.clone()), task(a.clone())],
					Box::new(task(a)),
				))
			}
			"manual-cancel" => {
				let a = self.new_type_var();
				Some(Type::Fun(
					vec![manual_scope_handle_type(a), Type::Nothing],
					Box::new(Type::Nothing),
				))
			}
			"manual-cancel-after" => {
				let a = self.new_type_var();
				Some(Type::Fun(
					vec![manual_scope_handle_type(a), Type::Duration],
					Box::new(Type::Nothing),
				))
			}
			"manual-next" => {
				let a = self.new_type_var();
				let e = self.new_type_var();
				let result_ty = Type::Enum("__prelude__.result".to_string(), vec![a.clone(), e]);
				let option_ty = Type::Enum("__prelude__.option".to_string(), vec![result_ty]);
				Some(Type::Fun(
					vec![manual_scope_handle_type(a), Type::Nothing],
					Box::new(task(option_ty)),
				))
			}
			_ => None,
		}
	}

	// Rewrite a scope-handle method call `s.method args` into a call to the
	// corresponding `task.scope-*` kernel builtin with `s` prepended:
	// `s.spawn t` -> `task.scope-spawn s t`. No-op for anything else.
	fn maybe_rewrite_scope_method(&mut self, expr: &mut ExprNode) {
		// Resolve to the kernel def name iff this is `handle.method args` with a
		// scope-handle receiver and a method valid for that handle kind.
		let def_name: Option<&'static str> = match &expr.kind {
			ExprKind::Call(call) => match &call.callee.kind {
				ExprKind::FieldAccess { receiver, field } => match &receiver.kind {
					ExprKind::Identifier(id) => self
						.handle_kind_of_binding(&id.name)
						.and_then(|kind| scope_method_def(&field.name, kind)),
					_ => None,
				},
				_ => None,
			},
			_ => None,
		};
		let def_name = match def_name {
			Some(d) => d,
			None => return,
		};

		let (callee, mut args, dict_args, crange) =
			match std::mem::replace(&mut expr.kind, ExprKind::EmptyTuple) {
				ExprKind::Call(CallNode {
					callee,
					args,
					dict_args,
					range,
				}) => (callee, args, dict_args, range),
				_ => unreachable!(),
			};
		let (receiver, field) = match callee.kind {
			ExprKind::FieldAccess { receiver, field } => (*receiver, field),
			_ => unreachable!(),
		};

		// New callee: the kernel def `<def_name>`. In a module that imports
		// `core.task` (under whatever local name) it lives in that namespace, so
		// emit a `<local>.<def_name>` namespace access with its type synthesized
		// here (the kernel defs are private to `core.task`, so they aren't in its
		// exports and can't be resolved through imports — `scope_kernel_def_type`
		// supplies the known signature). Synthesizing it here, rather than letting
		// the FieldAccess resolver look it up, is precisely what keeps these defs
		// private: a kernel name a *user* writes (`task.scope-spawn …`) still
		// reaches the resolver and is reported private. Inside `core.task` itself
		// there's no such import — the def is a local top-level — so reference it
		// bare (this is what lets the combinators in task.pa use `s.spawn`/`s.next`
		// on their own handles, and it resolves regardless of visibility).
		let task_local = self
			.import_qualified
			.iter()
			.find(|(_, full)| full.as_str() == "core.task")
			.map(|(local, _)| local.clone());
		let new_callee = match task_local {
			Some(local) => {
				let ty = self
					.scope_kernel_def_type("core.task", def_name)
					.expect("scope_method_def name must have a kernel signature");
				ExprNode {
					ty,
					range: field.range,
					kind: ExprKind::NamespaceAccess(vec![
						IdentifierNode {
							name: local,
							range: field.range,
						},
						IdentifierNode {
							name: def_name.to_string(),
							range: field.range,
						},
					]),
					trait_dispatch: None,
					dispatch_sink: None,
				}
			}
			None => ExprNode {
				ty: Type::Unknown,
				range: field.range,
				kind: ExprKind::Identifier(IdentifierNode {
					name: def_name.to_string(),
					range: field.range,
				}),
				trait_dispatch: None,
				dispatch_sink: None,
			},
		};

		let mut new_args = Vec::with_capacity(args.len() + 1);
		new_args.push(receiver);
		new_args.append(&mut args);

		expr.kind = ExprKind::Call(CallNode {
			range: crange,
			callee: Box::new(new_callee),
			args: new_args,
			dict_args,
		});
	}

	fn constrain_expr(&mut self, expr: &mut ExprNode, constraints: &mut Vec<Constraint>) {
		use Constraint::*;

		// `scope`-handle method calls (`s.spawn t`, `s.cancel ()`, …) rewrite
		// into calls to the hidden `task.scope-*` kernel builtins, with the
		// handle prepended as the first argument. Done before the main match so
		// the rewritten call is type-checked + lowered like any other call.
		self.maybe_rewrite_scope_method(expr);

		match &mut expr.kind {
			// For each of these, we don't bother introducing a new type var and generating
			// a constraint that the var is eq to the known concrete type. We could do that
			// (the algorithm would handle it fine), but assigning the concrete type directly
			// is nicer to look at and saves a couple steps.
			ExprKind::EmptyTuple => expr.ty = Type::Nothing,
			ExprKind::Regex(node) => {
				self.check_regex_character_classes(node);
				expr.ty = Type::Regex;
			}
			ExprKind::Literal(literal) => match &mut literal.kind {
				LiteralKind::Bool(..) => expr.ty = Type::Bool,
				LiteralKind::String(..) => expr.ty = Type::String,
				LiteralKind::Bytes(..) => expr.ty = Type::Bytes,
				LiteralKind::Duration(..) => expr.ty = Type::Duration,
				LiteralKind::FloatDecimal(..) => expr.ty = Type::Float,
				LiteralKind::IntDecimal(..)
				| LiteralKind::IntHex(..)
				| LiteralKind::IntBinary(..)
				| LiteralKind::IntOctal(..) => expr.ty = Type::Int,
			},

			ExprKind::Identifier(ident) => {
				if let Some(binding) = self.get_value_binding(&ident.name) {
					let ty_scheme = binding.ty_scheme.clone();
					return match ty_scheme {
						Scheme::Forall(vars, row_vars, class_constraints, ty) => {
							if vars.is_empty() && row_vars.is_empty() && class_constraints.is_empty() {
								expr.ty = ty;
							} else {
								// Polymorphic scheme. Freshen the quantified
								// vars per use site, and link via a fresh
								// expr-level var so post-unification
								// substitution reaches into the type
								// (fill_in_placeholder only resolves top-level
								// vars, not vars nested inside e.g. Fun).
								let (instantiated, fresh_constraints) = self.instantiate_scheme_with_constraints(
									&Scheme::Forall(vars, row_vars, class_constraints, ty),
								);
								let expr_ty = self.new_type_var();
								expr.ty = expr_ty.clone();
								constraints.push(eq_constraint(expr_ty, instantiated));
								// If the def carries class constraints (e.g. a
								// same-module `where (hash k)` def), set up a
								// dispatch sink so the surrounding Call threads
								// the dicts as `dict_args` — mirroring the
								// cross-module NamespaceAccess path. Without this
								// a local reference to a constrained def would
								// call it with no dict and the wrong arity.
								if !fresh_constraints.is_empty() {
									let sink = crate::ast::new_dispatch_sink();
									for class in fresh_constraints {
										sink.borrow_mut().push(class.dispatch_cell.clone());
										self.fresh_class_constraints.push(class.clone());
										constraints.push(Constraint::Class(class));
									}
									expr.dispatch_sink = Some(sink);
								}
							}
						}

						Scheme::Var(var) => {
							let expr_ty = self.new_type_var();
							expr.ty = expr_ty.clone();
							// Create a fresh sink for any class constraints
							// the matched scheme may carry — Gen processing
							// will push their cells in here, and the
							// surrounding Call reads them as dict_args.
							let sink = crate::ast::new_dispatch_sink();
							expr.dispatch_sink = Some(sink.clone());
							constraints.push(Inst(var, expr_ty, sink, expr.range));
						}
					};
				};

				// Bare variant constructor: `some 5` instead of `option.some 5`.
				// Look up the bare name in variant_constructors; resolve uniquely
				// or report ambiguity. Local-module variants shadow imported/
				// prelude ones with the same name.
				if let Some(matches) = self.variant_constructors.get(&ident.name).cloned() {
					let resolved = self.disambiguate_variant_matches(&matches);
					match resolved {
						Some((qualified_enum, variant_name)) => {
							if let Some(enum_def) = self.enum_defs.get(&qualified_enum).cloned() {
								let (enum_ty, variant_params, found) =
									self.instantiate_variant(&qualified_enum, &variant_name, &enum_def);
								if found.is_some() {
									if variant_params.is_empty() {
										expr.ty = enum_ty;
									} else {
										expr.ty = Type::Fun(variant_params, enum_ty.into());
									}
									return;
								}
							}
						}
						None => {
							let enums = matches.iter().map(|(q, _)| q.clone()).collect();
							self.error(
								ident.range,
								AmbiguousVariant {
									name: ident.name.clone(),
									enums,
								},
							);
							expr.ty = Type::Unknown;
							return;
						}
					}
				}

				// Bare trait method: `hash 42` instead of `hash.hash 42`.
				// Find all in-scope traits where the bare name is a method;
				// disambiguate by preferring module-local traits. Local
				// `def`s / variants shadow these (checked earlier).
				let method_matches: Vec<(String, usize, Type, usize)> = self
					.traits
					.iter()
					.filter_map(|(trait_name, decl)| {
						let idx = decl.method_order.iter().position(|m| m == &ident.name)?;
						let method_ty = decl.method_types.get(&ident.name)?.clone();
						Some((trait_name.clone(), idx, method_ty, decl.param_var))
					})
					.collect();

				if !method_matches.is_empty() {
					match self.disambiguate_bare_method_matches(&method_matches) {
						Some((trait_name, method_idx, method_ty, param_var)) => {
							self.emit_trait_method_dispatch(
								trait_name,
								method_idx,
								&method_ty,
								param_var,
								expr,
								constraints,
							);
							return;
						}
						None => {
							let traits = method_matches.iter().map(|(t, ..)| t.clone()).collect();
							self.error(
								ident.range,
								AmbiguousBareMethod {
									name: ident.name.clone(),
									traits,
								},
							);
							expr.ty = Type::Unknown;
							return;
						}
					}
				}

				self.error(
					ident.range,
					NameNotBound {
						name: ident.name.clone(),
					},
				);

				expr.ty = Type::Unknown;
			}

			ExprKind::Interpolation(parts) => {
				for part in parts {
					self.constrain_expr(part, constraints);

					// each part must have type string
					constraints.push(eq_constraint(part.ty.clone(), Type::String).at(part.range));
				}

				expr.ty = Type::String;
			}

			ExprKind::Grouping(inner) => {
				let expr_ty = self.new_type_var();
				expr.ty = expr_ty.clone();

				self.constrain_expr(inner, constraints);

				constraints.push(eq_constraint(expr_ty, inner.ty.clone()));
			}

			ExprKind::Tuple(elements) => {
				expr.ty = self.new_type_var();

				let mut element_types = Vec::new();

				for element in elements {
					self.constrain_expr(element, constraints);
					element_types.push(element.ty.clone());
				}

				constraints.push(eq_constraint(expr.ty.clone(), Type::Tuple(element_types)).at(expr.range))
			}

			ExprKind::List(items) => {
				// All elements must share a type. Empty list gets a fresh
				// element-type var so the overall type is `list 'a`. The expr
				// type is itself a fresh Var equated to `list elem_ty` so the
				// post-unification substitution can resolve it (fill_in_placeholder
				// only descends into top-level Vars). A plain item has the
				// element type; a `...spread` is itself a `list element_ty`.
				expr.ty = self.new_type_var();
				let element_type = self.new_type_var();
				for item in items {
					let is_spread = item.is_spread();
					let inner = item.expr_mut();
					self.constrain_expr(inner, constraints);
					let expected = if is_spread {
						Type::List(Box::new(element_type.clone()))
					} else {
						element_type.clone()
					};
					constraints.push(eq_constraint(inner.ty.clone(), expected).at(inner.range));
				}
				constraints
					.push(eq_constraint(expr.ty.clone(), Type::List(Box::new(element_type))).at(expr.range));
			}

			ExprKind::Record(fields) => {
				expr.ty = self.new_type_var();

				let mut field_types = Vec::new();

				for (field_name, field_value) in fields {
					self.constrain_expr(field_value, constraints);
					field_types.push((field_name.name.clone(), field_value.ty.clone()));
				}

				constraints
					.push(eq_constraint(expr.ty.clone(), Type::Record(field_types, None)).at(expr.range))
			}

			ExprKind::RecordUpdate { base, fields } => {
				// Update-only, type-preserving (Elm-style). Constrain `base` to
				// be an *open* record carrying at least the override fields, each
				// at its override value's type. Because the record is open, the
				// only way unification succeeds is if `base` already has those
				// fields at exactly those types — that rejects type-changing
				// overrides and brand-new fields, while leaving the untouched
				// fields in the row tail. The result type is just `base`'s.
				self.constrain_expr(base, constraints);

				let mut field_types = Vec::new();
				for (field_name, field_value) in fields {
					self.constrain_expr(field_value, constraints);
					field_types.push((field_name.name.clone(), field_value.ty.clone()));
				}

				let rid = self.new_row_var();
				constraints.push(
					eq_constraint(base.ty.clone(), Type::Record(field_types, Some(rid))).at(expr.range),
				);

				expr.ty = base.ty.clone();
			}

			ExprKind::UnaryOperation { op, right } => {
				self.constrain_expr(right, constraints);
				match op {
					Operator::SubtractionOrNegation => {
						// Unary `-` desugars to the `numeric.negate` trait method:
						// fresh tyvar α for the dispatch type; constrain operand
						// and result to α; emit a Class constraint that picks
						// the int/float instance once unification resolves α.
						let alpha = self.new_type_var();
						expr.ty = alpha.clone();
						constraints.push(eq_constraint(right.ty.clone(), alpha.clone()).at(right.range));
						self.emit_numeric_dispatch(expr, "negate", &alpha, constraints);
					}
					Operator::LogicalNot => {
						expr.ty = Type::Bool;
						constraints.push(eq_constraint(right.ty.clone(), Type::Bool).at(right.range));
					}
					_ => {
						// Other prefix ops not supported yet.
					}
				}
			}

			ExprKind::BinaryOperation { left, right, op } => {
				// `x | f a b` pipes `x` as the first arg of the RHS call: `f x a b`.
				// We don't visit `right` as a normal expression because its standalone
				// type (a Call's return type with the wrong arity) would conflict with
				// the prepended-arg signature we want.
				if let Operator::Chain = op.kind {
					expr.ty = self.new_type_var();
					self.constrain_expr(left, constraints);

					match &mut right.kind {
						ExprKind::Call(CallNode { callee, args, .. }) => {
							self.constrain_expr(callee, constraints);
							let mut arg_types = vec![left.ty.clone()];
							for arg in args.iter_mut() {
								self.constrain_expr(arg, constraints);
								arg_types.push(arg.ty.clone());
							}
							constraints.push(
								eq_constraint(
									callee.ty.clone(),
									Type::Fun(arg_types, expr.ty.clone().into()),
								)
								.at(expr.range),
							);
							right.ty = expr.ty.clone();
						}
						_ => {
							self.constrain_expr(right, constraints);
							constraints.push(
								eq_constraint(
									right.ty.clone(),
									Type::Fun(vec![left.ty.clone()], expr.ty.clone().into()),
								)
								.at(expr.range),
							);
						}
					}

					return;
				}

				self.constrain_expr(left, constraints);
				self.constrain_expr(right, constraints);

				match &op.kind {
					Operator::Addition
					| Operator::SubtractionOrNegation
					| Operator::Multiplication
					| Operator::Division => {
						// Arithmetic operators desugar to `numeric.*` trait
						// method dispatch. Fresh tyvar α for the dispatch type;
						// constrain both sides + result to α; emit a Class
						// constraint that picks int/float once unification
						// resolves α. Stays polymorphic if α survives:
						// `def double fun x { x + x }` becomes
						// `forall a. Numeric a => a -> a`.
						let alpha = self.new_type_var();
						expr.ty = alpha.clone();
						constraints.push(eq_constraint(left.ty.clone(), alpha.clone()).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), alpha.clone()).at(right.range));
						let method = match op.kind {
							Operator::Addition => "add",
							Operator::SubtractionOrNegation => "sub",
							Operator::Multiplication => "mul",
							Operator::Division => "div",
							_ => unreachable!(),
						};
						self.emit_numeric_dispatch(expr, method, &alpha, constraints);
					}

					Operator::Remainder => {
						// `%` stays int-only for now — the plan defers it to a
						// future `integral` trait. Keep the legacy heuristic so
						// existing remainder uses still resolve.
						let is_float = matches!(left.ty, Type::Float) || matches!(right.ty, Type::Float);
						let ty = if is_float { Type::Float } else { Type::Int };
						expr.ty = ty.clone();
						constraints.push(eq_constraint(left.ty.clone(), ty.clone()).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), ty).at(right.range));
					}

					Operator::LogicalAnd | Operator::LogicalOr => {
						expr.ty = Type::Bool;
						constraints.push(eq_constraint(left.ty.clone(), Type::Bool).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), Type::Bool).at(right.range));
					}

					// `++` concatenates two strings into a string. No trait
					// dispatch — both sides are pinned to `string` and codegen
					// lowers it to a single `ConcatString` instruction.
					Operator::Concat => {
						expr.ty = Type::String;
						constraints.push(eq_constraint(left.ty.clone(), Type::String).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), Type::String).at(right.range));
					}

					// `==`/`!=` are polymorphic but require both sides to match.
					// Result type is bool either way.
					Operator::Equality | Operator::Inequality => {
						expr.ty = Type::Bool;
						constraints.push(eq_constraint(left.ty.clone(), right.ty.clone()).at(expr.range));
					}

					// Ordering desugars to `ord.compare` plus a comparison
					// with one of the `ordering` variants:
					//   `a < b`  -> `ord.compare a b == lt`
					//   `a > b`  -> `ord.compare a b == gt`
					//   `a <= b` -> `ord.compare a b != gt`
					//   `a >= b` -> `ord.compare a b != lt`
					// The analyzer just sets up the trait dispatch on the
					// BinaryOp expression; codegen emits the variant-eq tail.
					Operator::LessThan
					| Operator::LessThanEquals
					| Operator::GreaterThan
					| Operator::GreaterThanEquals => {
						let alpha = self.new_type_var();
						expr.ty = Type::Bool;
						constraints.push(eq_constraint(left.ty.clone(), alpha.clone()).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), alpha.clone()).at(right.range));
						self.emit_ord_dispatch(expr, &alpha, constraints);
					}

					Operator::FieldAccess => unreachable!("handled separately"),

					// `lhs ?? default` unwraps an option/result to a bare value,
					// substituting `default` on the empty/error arm. The carrier
					// (option vs result) can't be known until unification, so —
					// like `try` — we only set up the carrier-independent facts
					// here: the result type, the default's type, and the
					// unwrapped payload all coincide. The post-unify dispatch
					// pass reads the resolved LHS and rewrites this node into a
					// `<carrier>.or-else` call.
					Operator::NullCoalescing => {
						let alpha = self.new_type_var();
						expr.ty = alpha.clone();
						constraints.push(eq_constraint(right.ty.clone(), alpha).at(right.range));
					}

					_ => {
						// Other binary ops not supported yet.
					}
				}
			}

			ExprKind::Fun(FunNode { params, body, .. }) => {
				expr.ty = self.new_type_var();

				// Consume any signature hint (set for an annotated def's RHS). A
				// param whose hinted type is a scope handle is bound to that
				// concrete type rather than a fresh var, so handle methods dispatch
				// on it; everything else stays a fresh var (unified with the
				// annotation as usual). Taking it means nested funs don't inherit.
				let hints = self.fun_param_hints.take();

				let mut param_types = Vec::new();

				self.enter_scope();

				if params.is_empty() {
					param_types.push(Type::Nothing)
				} else {
					for (i, param) in params.iter_mut().enumerate() {
						param.ty = match hints.as_ref().and_then(|h| h.get(i)) {
							Some(t) if is_handle_type(t) => t.clone(),
							_ => self.new_type_var(),
						};

						param_types.push(param.ty.clone());

						self.add_value_binding(
							param.ident.name.clone(),
							Scheme::Forall(vec![], vec![], vec![], param.ty.clone()),
							param.ident.range,
						)
					}
				}

				let mut return_type = Type::Nothing;

				for expr in body {
					self.constrain_expr(expr, constraints);
					return_type = expr.ty.clone();
				}

				self.leave_scope();

				// we know that this lambda must be a function that takes
				// the param types and returns the return type
				constraints.push(
					eq_constraint(
						expr.ty.clone(),
						Type::Fun(param_types, Box::new(return_type)),
					)
					.at(expr.range),
				);
			}

			ExprKind::Call(CallNode {
				callee,
				args,
				dict_args,
				..
			}) => {
				expr.ty = self.new_type_var();

				self.constrain_expr(callee, constraints);

				// If the callee is a polymorphic constrained value reference,
				// its `dispatch_sink` will be populated by Gen/Inst processing
				// during unify. We capture the sink here so codegen can read
				// dict_args from it after analysis finishes. (The cells in
				// the sink may not be filled until unify runs — we hold the
				// Rc so we still see them when annotate / codegen runs.)
				if let Some(sink) = callee.dispatch_sink.clone() {
					// `dict_args` will be hydrated from `sink` at annotate
					// time. Stash the sink on the callee — annotate reads
					// it back. We leave `dict_args` empty here.
					let _ = sink;
					let _ = &dict_args; // (populated during annotate)
				}

				let mut arg_types = Vec::new();

				for arg in args {
					self.constrain_expr(arg, constraints);
					arg_types.push(arg.ty.clone());
				}

				// we know that the callee should be a function that takes
				// the given arg types and returns the type of this whole expr
				constraints.push(
					eq_constraint(
						callee.ty.clone(),
						Type::Fun(arg_types, expr.ty.clone().into()),
					)
					.at(expr.range),
				);
			}

			ExprKind::Try(TryNode {
				pattern,
				value,
				rest,
				pattern_ty,
				..
			}) => {
				// First-pass constrain for `try`. We constrain the value and
				// the rest (in a scope where `pattern` binds to a fresh `α`),
				// but we DO NOT yet link `α` to the value's payload type or
				// `expr.ty` to a carrier-wrapped shape — that depends on the
				// RHS's inferred head constructor, which isn't known until
				// after unify. The post-unify `dispatch_try_nodes` pass
				// reads the head, picks a carrier, rewrites this node into
				// a `<carrier>.then` call, and emits the remaining linking
				// constraints (which are then re-unified).
				expr.ty = self.new_type_var();

				self.constrain_expr(value, constraints);

				self.enter_scope();
				let alpha = self.new_type_var();
				*pattern_ty = alpha.clone();
				self.constrain_let_pattern(pattern, alpha.clone(), constraints);
				for r in rest.iter_mut() {
					self.constrain_expr(r, constraints);
				}
				self.leave_scope();
			}

			ExprKind::Let(LetNode {
				pattern,
				value,
				type_annotation,
				..
			}) => {
				// visit the value (expression after the `=`), and collect constraints:
				self.constrain_expr(value, constraints);

				// `:: TYPE` annotation — same shape as the top-level def
				// form. Resolve in the value's current scope (free names
				// in the annotation introduce fresh type vars) and unify
				// with the bound expression's inferred type.
				if let Some(annotation) = type_annotation {
					let (annotated_ty, _) = self.resolve_annotation(annotation, constraints);
					constraints.push(eq_constraint(value.ty.clone(), annotated_ty).at(annotation.range));
				}

				match &mut pattern.kind {
					PatternKind::Identifier(name) => {
						// Mono-vs-poly. Generalize (Gen/Inst, so the binding can be
						// polymorphic) only when BOTH the value's type still has free
						// vars AND the RHS is a syntactic value — the ML *value
						// restriction*. A non-value RHS (a function application like
						// `s.spawn t`, `ref.new x`, …) binds monomorphically: its free
						// vars are determined by the surrounding context, so quantifying
						// them would wrongly decouple the binding from its inputs (and is
						// unsound for effectful/aliasing results). A concrete-typed value
						// also binds monomorphically so later uses see the resolved type.
						if value.ty.free_vars().is_empty() || !is_syntactic_value(value) {
							self.add_value_binding(
								name.name.clone(),
								Scheme::Forall(vec![], vec![], vec![], value.ty.clone()),
								name.range,
							);
						} else {
							let type_scheme = self.new_type_scheme_var();
							self.add_value_binding(name.name.clone(), type_scheme.clone(), name.range);
							constraints.push(Gen(type_scheme, value.ty.clone()));
						}
					}
					_ => {
						let subject_ty = value.ty.clone();
						self.constrain_let_pattern(pattern, subject_ty, constraints);
					}
				}

				// let expressions always evaluate to ()
				expr.ty = Type::Nothing;
			}

			ExprKind::Defer(inner) => {
				// The deferred expression's value is discarded (it runs at
				// function exit for its effects), so it carries no constraint
				// beyond being internally well-typed. `defer` itself is `nothing`.
				self.constrain_expr(inner, constraints);
				expr.ty = Type::Nothing;
			}

			ExprKind::Scope(_) => {
				// Bind the handle to `scope-handle` (monomorphic) so method
				// calls on it dispatch at constrain time, then constrain the
				// body like a function body. The body must produce a `task a`;
				// the whole `scope` expression has that `task a` type.
				let (manual, handle, mut body, srange) =
					match std::mem::replace(&mut expr.kind, ExprKind::EmptyTuple) {
						ExprKind::Scope(ScopeNode {
							manual,
							handle,
							body,
							range,
						}) => (manual, handle, body, range),
						_ => unreachable!(),
					};

				expr.ty = self.new_type_var();

				self.enter_scope();
				if let Some(h) = &handle {
					// Fail-fast `scope` binds an unparameterized `scope-handle`
					// (heterogeneous children); `manual scope` binds a
					// `manual-scope-handle a` whose `a` is fixed by `s.spawn`/`s.next`.
					let handle_ty = if manual {
						manual_scope_handle_type(self.new_type_var())
					} else {
						scope_handle_type()
					};
					self.add_value_binding(
						h.name.clone(),
						Scheme::Forall(vec![], vec![], vec![], handle_ty),
						h.range,
					);
				}

				let mut body_ty = Type::Nothing;
				for e in body.iter_mut() {
					self.constrain_expr(e, constraints);
					body_ty = e.ty.clone();
				}
				self.leave_scope();

				// The body must produce a task; the scope expression is that task.
				let alpha = self.new_type_var();
				let task_ty = Type::Enum("__prelude__.task".to_string(), vec![alpha]);
				constraints.push(eq_constraint(body_ty, task_ty.clone()).at(srange));
				constraints.push(eq_constraint(expr.ty.clone(), task_ty).at(srange));

				expr.kind = ExprKind::Scope(ScopeNode {
					range: srange,
					manual,
					handle,
					body,
				});
			}

			ExprKind::ElementAccess { receiver, index } => {
				// this expr gets a fresh type var
				expr.ty = self.new_type_var();

				self.constrain_expr(receiver, constraints);

				// we know that receiver is a "partial tuple": at the given index,
				// it must have a value of the type of this expr. The fresh row
				// var leaves the rest of the tuple open, so several accesses on
				// the same value merge their indices through unification (the
				// tuple analogue of open-record field access).
				let row = self.new_row_var();
				constraints.push(
					eq_constraint(
						receiver.ty.clone(),
						Type::PartialTuple(vec![(*index, expr.ty.clone())], Some(row)),
					)
					.at(expr.range),
				)
			}

			ExprKind::FieldAccess { .. } => {
				// Take ownership of the FieldAccess contents so we can freely
				// reshape `expr.kind` into `NamespaceAccess` below without
				// fighting the borrow checker. If none of the namespace
				// cases apply, we put the FieldAccess back for the record
				// field-access fallback.
				let (mut receiver, field) = match std::mem::replace(&mut expr.kind, ExprKind::EmptyTuple) {
					ExprKind::FieldAccess { receiver, field } => (receiver, field),
					_ => unreachable!(),
				};

				// Cross-module variant access: `module.enum-name.variant`.
				// Match the chained-FieldAccess shape so we resolve before the
				// inner receiver gets recursed into as a regular field access.
				if let ExprKind::FieldAccess {
					receiver: outer_recv,
					field: enum_field,
				} = &receiver.kind
				{
					if let ExprKind::Identifier(module_ident) = &outer_recv.kind {
						if let Some(exports) = self.imports.get(&module_ident.name).cloned() {
							if exports.enums.contains_key(&enum_field.name) {
								let qualified_module = self
									.import_qualified
									.get(&module_ident.name)
									.cloned()
									.unwrap_or_else(|| module_ident.name.clone());
								let qualified = format!("{}.{}", qualified_module, enum_field.name);
								if let Some(enum_def) = self.enum_defs.get(&qualified).cloned() {
									let (enum_ty, variant_params, variant_found) =
										self.instantiate_variant(&qualified, &field.name, &enum_def);
									let path = vec![module_ident.clone(), enum_field.clone(), field.clone()];
									expr.kind = ExprKind::NamespaceAccess(path);

									match variant_found {
										Some(_) if variant_params.is_empty() => {
											expr.ty = enum_ty;
										}
										Some(_) => {
											expr.ty = Type::Fun(variant_params, enum_ty.into());
										}
										None => {
											self.error(
												field.range,
												EnumVariantNotPresent {
													variant: field.name.clone(),
													ty: enum_ty,
												},
											);
											expr.ty = Type::Unknown;
										}
									}
									return;
								}
							}
						}
					}
				}

				// Module namespace access: `module-name.def`. If the receiver
				// is a bare ident that matches an imported module, look up
				// the field in that module's exported values. If the module
				// doesn't have the field and the same ident is also a known
				// enum (e.g. the auto-imported `option` module overlapping
				// with the prelude `option` enum), fall through to the local
				// variant-access case below.
				if let ExprKind::Identifier(ident) = &receiver.kind {
					if let Some(exports) = self.imports.get(&ident.name).cloned() {
						match exports.values.get(&field.name) {
							Some(ty) => {
								expr.kind = ExprKind::NamespaceAccess(vec![ident.clone(), field.clone()]);
								// Freshen the value's type *and* its class
								// constraints together so they share fresh
								// tyvars. Each constraint becomes a fresh
								// Class constraint with a fresh dispatch
								// cell; the cell ends up in the surrounding
								// Call's dict_args via the dispatch_sink.
								let mut mapping: HashMap<usize, Type> = HashMap::new();
								let fresh_ty = self.instantiate_with(ty, &mut mapping);
								expr.ty = fresh_ty;
								if let Some(constraints_export) = exports.value_constraints.get(&field.name) {
									let sink = crate::ast::new_dispatch_sink();
									for vc in constraints_export {
										let fresh_var = self.instantiate_with(&vc.dispatch_var, &mut mapping);
										let cell =
											crate::ast::new_dispatch(vc.trait_name.clone(), None, fresh_var.clone());
										sink.borrow_mut().push(cell.clone());
										let class = ClassConstraint {
											name: vc.trait_name.clone(),
											ty: fresh_var,
											reason: ConstraintReason { range: expr.range },
											dispatch_cell: cell,
										};
										self.fresh_class_constraints.push(class.clone());
										constraints.push(Constraint::Class(class));
									}
									expr.dispatch_sink = Some(sink);
								}
								return;
							}
							None => {
								let is_local_enum = self
									.type_scope
									.get(&ident.name)
									.map(|b| matches!(b.ty, Type::Enum(_, _)))
									.unwrap_or(false);
								if !is_local_enum {
									if exports.private.contains(&field.name) {
										self.error(
											field.range,
											ItemPrivate {
												name: field.name.clone(),
												module: ident.name.clone(),
											},
										);
									} else {
										self.error(
											field.range,
											NameNotBound {
												name: format!("{}.{}", ident.name, field.name),
											},
										);
									}
									expr.ty = Type::Unknown;
									return;
								}
								// Fall through: the local-variant case below
								// will resolve `field` against the enum and
								// emit a precise diagnostic on miss.
							}
						}
					}
				}

				// Trait method access: `trait-name.method`. The receiver is
				// a bare ident that matches a registered typeclass. Resolve
				// the method via `emit_trait_method_dispatch`, which sets up
				// the shared dispatch cell + Class constraint. The bare-name
				// form (`method` without a trait prefix) is handled
				// alongside identifier resolution.
				if let ExprKind::Identifier(ident) = &receiver.kind {
					if let Some(trait_decl) = self.traits.get(&ident.name) {
						let trait_name = ident.name.clone();
						let param_var = trait_decl.param_var;
						let method_idx = trait_decl
							.method_order
							.iter()
							.position(|m| m == &field.name);
						let method_type = trait_decl.method_types.get(&field.name).cloned();

						match (method_idx, method_type) {
							(Some(idx), Some(method_ty)) => {
								expr.kind = ExprKind::NamespaceAccess(vec![ident.clone(), field.clone()]);
								self.emit_trait_method_dispatch(
									trait_name,
									idx,
									&method_ty,
									param_var,
									expr,
									constraints,
								);
								return;
							}
							_ => {
								self.error(
									field.range,
									NameNotBound {
										name: format!("{}.{}", ident.name, field.name),
									},
								);
								expr.ty = Type::Unknown;
								return;
							}
						}
					}
				}

				// Local variant access: `EnumName.variant`. The receiver is a
				// bare ident that resolves (via type_scope) to a known enum.
				if let ExprKind::Identifier(ident) = &receiver.kind {
					let qualified_enum = self.type_scope.get(&ident.name).and_then(|binding| {
						if let Type::Enum(name, _) = &binding.ty {
							Some(name.clone())
						} else {
							None
						}
					});

					if let Some(qualified) = qualified_enum {
						if let Some(enum_def) = self.enum_defs.get(&qualified).cloned() {
							let (enum_ty, variant_params, variant_found) =
								self.instantiate_variant(&qualified, &field.name, &enum_def);
							expr.kind = ExprKind::NamespaceAccess(vec![ident.clone(), field.clone()]);

							match variant_found {
								Some(_) if variant_params.is_empty() => {
									expr.ty = enum_ty;
								}
								Some(_) => {
									expr.ty = Type::Fun(variant_params, enum_ty.into());
								}
								None => {
									self.error(
										field.range,
										EnumVariantNotPresent {
											variant: field.name.clone(),
											ty: enum_ty,
										},
									);
									expr.ty = Type::Unknown;
								}
							}

							return;
						}
					}
				}

				// None of the namespace cases applied — this is a real record
				// field access. Put the FieldAccess back together so later
				// passes (annotate_expr, codegen) still see the right shape,
				// and emit the row-polymorphic record constraint.
				expr.ty = self.new_type_var();
				self.constrain_expr(&mut receiver, constraints);

				let rid = self.new_row_var();
				constraints.push(
					eq_constraint(
						receiver.ty.clone(),
						Type::Record(vec![(field.name.clone(), expr.ty.clone())], Some(rid)),
					)
					.at(expr.range),
				);

				expr.kind = ExprKind::FieldAccess { receiver, field };
			}

			ExprKind::If(IfNode {
				subject,
				pattern,
				body,
				else_body,
				..
			}) => {
				self.constrain_expr(subject, constraints);

				self.enter_scope();
				self.constrain_pattern(pattern, subject.ty.clone(), constraints);
				let mut body_ty = Type::Nothing;
				for body_expr in body.iter_mut() {
					self.constrain_expr(body_expr, constraints);
					body_ty = body_expr.ty.clone();
				}
				self.leave_scope();

				match else_body {
					Some(else_body) => {
						// With `else`, the if is a value expression: both branch
						// types must agree, and the if takes that type.
						let mut else_ty = Type::Nothing;
						for else_expr in else_body.iter_mut() {
							self.constrain_expr(else_expr, constraints);
							else_ty = else_expr.ty.clone();
						}
						expr.ty = self.new_type_var();
						constraints.push(eq_constraint(expr.ty.clone(), body_ty).at(expr.range));
						constraints.push(eq_constraint(expr.ty.clone(), else_ty).at(expr.range));
					}
					None => {
						// Single-armed if always evaluates to nothing.
						expr.ty = Type::Nothing;
					}
				}
			}

			ExprKind::While(WhileNode {
				subject,
				pattern,
				body,
				..
			}) => {
				self.constrain_expr(subject, constraints);

				self.enter_scope();
				self.constrain_pattern(pattern, subject.ty.clone(), constraints);
				for body_expr in body.iter_mut() {
					self.constrain_expr(body_expr, constraints);
				}
				self.leave_scope();

				expr.ty = Type::Nothing;
			}

			ExprKind::When(WhenNode { subject, cases, .. }) => {
				self.constrain_expr(subject, constraints);
				expr.ty = self.new_type_var();

				for case in cases.iter_mut() {
					self.enter_scope();
					self.constrain_pattern(&mut case.pattern, subject.ty.clone(), constraints);

					let mut case_ty = Type::Nothing;
					for body_expr in case.body.iter_mut() {
						self.constrain_expr(body_expr, constraints);
						case_ty = body_expr.ty.clone();
					}
					self.leave_scope();

					constraints.push(eq_constraint(expr.ty.clone(), case_ty).at(case.range));
				}
			}

			ExprKind::NamespaceAccess(_) => {
				// constrain normally produces NamespaceAccess from a FieldAccess
				// receiver and never sees one as input — except the scope-method
				// rewrite, which emits a *pre-typed* `task.scope-*` kernel access
				// (those defs are private, so they can't be resolved through
				// imports — see `maybe_rewrite_scope_method`). Such a node already
				// carries its synthesized type, so leave it. Anything else is a bug.
				if matches!(expr.ty, Type::Unknown) {
					unreachable!("NamespaceAccess fed back into constrain_expr");
				}
			}

			ExprKind::Builtin(_) => {
				// `built-in` is only legal as the immediate RHS of a
				// type-annotated top-level def, which is handled
				// up in the value-def loop above. Anywhere else is a
				// misuse — diagnose and give it a fresh tyvar so the
				// rest of unification can proceed.
				self.error(expr.range, BuiltinMustBeTopLevelRhs);
				expr.ty = self.new_type_var();
			}
		}
	}

	// Constrain a `let`-binding pattern, which must be irrefutable. Mirrors
	// the shape of `constrain_pattern` for the irrefutable cases (Identifier,
	// Underscore, Tuple, Record) but skips variant-name resolution so a bare
	// ident always binds (e.g. `let (a, b) = ...` never tries to read `a` as
	// a nullary variant). Refutable patterns (Constructor, Literal,
	// Interpolation) are rejected with a diagnostic.
	fn constrain_let_pattern(
		&mut self,
		pattern: &mut PatternNode,
		subject_ty: Type,
		constraints: &mut Vec<Constraint>,
	) {
		match &mut pattern.kind {
			PatternKind::Underscore => {}

			PatternKind::Identifier(ident) => {
				self.add_value_binding(
					ident.name.clone(),
					Scheme::Forall(vec![], vec![], vec![], subject_ty),
					ident.range,
				);
			}

			PatternKind::Tuple(entries) => {
				let mut entry_types = Vec::new();
				for _ in entries.iter() {
					entry_types.push(self.new_type_var());
				}
				constraints
					.push(eq_constraint(subject_ty, Type::Tuple(entry_types.clone())).at(pattern.range));
				for (entry, entry_ty) in entries.iter_mut().zip(entry_types.into_iter()) {
					self.constrain_let_pattern(entry, entry_ty, constraints);
				}
			}

			PatternKind::Record { fields, rest } => {
				report_duplicate_record_pattern_fields(self, fields);
				// Build per-field fresh type vars first.
				let mut field_types = Vec::with_capacity(fields.len());
				let mut child_tys = Vec::with_capacity(fields.len());
				for (field_name, _) in fields.iter() {
					let ty = self.new_type_var();
					field_types.push((field_name.name.clone(), ty.clone()));
					child_tys.push(ty);
				}
				// No `...` → closed record. `...` → open record sharing a
				// fresh row variable; if the rest is named, bind it to a
				// record with no known fields and the same row variable, so
				// unification with a concrete subject resolves the rest to
				// "the leftover fields".
				let tail = match rest {
					None => None,
					Some(rp) => {
						let rid = self.new_row_var();
						if let Some(name) = &rp.binding {
							self.add_value_binding(
								name.name.clone(),
								Scheme::Forall(vec![], vec![], vec![], Type::Record(vec![], Some(rid))),
								name.range,
							);
						}
						Some(rid)
					}
				};
				constraints.push(
					eq_constraint(subject_ty.clone(), Type::Record(field_types, tail)).at(pattern.range),
				);
				for ((_, field_pattern), child_ty) in fields.iter_mut().zip(child_tys.into_iter()) {
					self.constrain_let_pattern(field_pattern, child_ty, constraints);
				}
			}

			PatternKind::List { items, rest } => {
				// List patterns can fail at runtime (the length might not
				// match), so they're refutable in `let` — except for
				// `[...]` or `[...rest]` with no required items, which
				// always matches.
				let always_succeeds = items.is_empty() && rest.is_some();
				if !always_succeeds {
					self.error(pattern.range, RefutablePatternInLet);
					return;
				}
				let elem_ty = self.new_type_var();
				constraints
					.push(eq_constraint(subject_ty.clone(), Type::List(Box::new(elem_ty))).at(pattern.range));
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						self.add_value_binding(
							name.name.clone(),
							Scheme::Forall(vec![], vec![], vec![], subject_ty),
							name.range,
						);
					}
				}
			}

			PatternKind::Constructor(..) | PatternKind::Literal(..) | PatternKind::Interpolation(..) => {
				self.error(pattern.range, RefutablePatternInLet);
			}
		}
	}

	fn constrain_pattern(
		&mut self,
		pattern: &mut PatternNode,
		subject_ty: Type,
		constraints: &mut Vec<Constraint>,
	) {
		match &mut pattern.kind {
			PatternKind::Underscore => {}

			PatternKind::Literal(literal) => {
				let lit_ty = match &literal.kind {
					LiteralKind::Bool(..) => Type::Bool,
					LiteralKind::String(..) => Type::String,
					LiteralKind::Bytes(..) => Type::Bytes,
					LiteralKind::Duration(..) => Type::Duration,
					LiteralKind::FloatDecimal(..) => Type::Float,
					LiteralKind::IntDecimal(..)
					| LiteralKind::IntHex(..)
					| LiteralKind::IntBinary(..)
					| LiteralKind::IntOctal(..) => Type::Int,
				};
				constraints.push(eq_constraint(subject_ty, lit_ty).at(pattern.range));
			}

			PatternKind::Identifier(ident) => {
				// A bare ident might be a nullary variant match. Use the subject
				// type to disambiguate; otherwise require global uniqueness.
				match self.resolve_variant_pattern(ident, &subject_ty, /* nullary_only */ true) {
					VariantResolution::Found(enum_name, _) => {
						let enum_ty = self.resolve_subject_enum_type(&subject_ty, &enum_name);
						constraints.push(eq_constraint(subject_ty, enum_ty).at(pattern.range));
					}
					VariantResolution::Ambiguous => {
						// error already reported
					}
					VariantResolution::NotFound => {
						self.add_value_binding(
							ident.name.clone(),
							Scheme::Forall(vec![], vec![], vec![], subject_ty),
							ident.range,
						);
					}
				}
			}

			PatternKind::Constructor(name, args) => {
				match self.resolve_variant_pattern(name, &subject_ty, /* nullary_only */ false) {
					VariantResolution::Found(enum_name, params) => {
						let enum_ty = self.resolve_subject_enum_type(&subject_ty, &enum_name);
						let enum_args = match &enum_ty {
							Type::Enum(_, args) => args.clone(),
							_ => Vec::new(),
						};
						constraints.push(eq_constraint(subject_ty, enum_ty).at(pattern.range));

						if args.len() != params.len() {
							self.error(
								pattern.range,
								ParamCountMismatch {
									expected: params.len(),
									found: args.len(),
								},
							);
							return;
						}

						// Substitute the enum's param vars with the subject's
						// concrete args before recursing — `some x` against
						// `option int` should bind `x: int`, not `x: Var(a)`.
						let param_vars = self
							.enum_defs
							.get(&enum_name)
							.map(|d| d.param_vars.clone())
							.unwrap_or_default();
						let subst = Substitution {
							solutions: param_vars.into_iter().zip(enum_args.into_iter()).collect(),
							row_solutions: HashMap::new(),
							tuple_row_solutions: HashMap::new(),
						};
						for (arg, param_ty) in args.iter_mut().zip(params.into_iter()) {
							self.constrain_pattern(arg, subst.apply_to_type(&param_ty), constraints);
						}
					}
					VariantResolution::Ambiguous => {
						// error already reported
					}
					VariantResolution::NotFound => {
						self.error(
							name.range,
							NameNotBound {
								name: name.name.clone(),
							},
						);
					}
				}
			}

			PatternKind::Tuple(entries) => {
				let mut entry_types = Vec::new();
				for _ in entries.iter() {
					entry_types.push(self.new_type_var());
				}
				constraints
					.push(eq_constraint(subject_ty, Type::Tuple(entry_types.clone())).at(pattern.range));
				for (entry, entry_ty) in entries.iter_mut().zip(entry_types.into_iter()) {
					self.constrain_pattern(entry, entry_ty, constraints);
				}
			}

			PatternKind::Record { fields, rest } => {
				report_duplicate_record_pattern_fields(self, fields);
				let mut field_types = Vec::with_capacity(fields.len());
				let mut child_tys = Vec::with_capacity(fields.len());
				for (field_name, _) in fields.iter() {
					let ty = self.new_type_var();
					field_types.push((field_name.name.clone(), ty.clone()));
					child_tys.push(ty);
				}
				let tail = match rest {
					None => None,
					Some(rp) => {
						let rid = self.new_row_var();
						if let Some(name) = &rp.binding {
							self.add_value_binding(
								name.name.clone(),
								Scheme::Forall(vec![], vec![], vec![], Type::Record(vec![], Some(rid))),
								name.range,
							);
						}
						Some(rid)
					}
				};
				constraints.push(
					eq_constraint(subject_ty.clone(), Type::Record(field_types, tail)).at(pattern.range),
				);
				for ((_, field_pattern), child_ty) in fields.iter_mut().zip(child_tys.into_iter()) {
					self.constrain_pattern(field_pattern, child_ty, constraints);
				}
			}

			PatternKind::List { items, rest } => {
				// Subject must be `list a` for some element type `a`. Each
				// item pattern matches against `a`. A named rest binding
				// captures the remainder as `list a`.
				let elem_ty = self.new_type_var();
				constraints.push(
					eq_constraint(subject_ty.clone(), Type::List(Box::new(elem_ty.clone())))
						.at(pattern.range),
				);
				for item in items.iter_mut() {
					self.constrain_pattern(item, elem_ty.clone(), constraints);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						self.add_value_binding(
							name.name.clone(),
							Scheme::Forall(vec![], vec![], vec![], Type::List(Box::new(elem_ty))),
							name.range,
						);
					}
				}
			}

			PatternKind::Interpolation(_) => {
				// TODO: interpolation patterns
			}
		}
	}

	fn check_when_exhaustive(&mut self, subject_ty: &Type, cases: &[CaseNode], range: Range) {
		if matches!(subject_ty, Type::List(_)) {
			return self.check_when_list_exhaustive(cases, range);
		}
		if matches!(subject_ty, Type::Record(_, _)) {
			return self.check_when_record_exhaustive(cases, range);
		}

		let required: Vec<String> = match subject_ty {
			Type::Bool => vec!["true".into(), "false".into()],
			Type::Enum(name, _) => match self.enum_defs.get(name) {
				Some(enum_def) => enum_def.variants.iter().map(|(n, _)| n.clone()).collect(),
				None => return,
			},
			// Other subject types are an "open universe" (e.g. int, string,
			// records, tuples) — exhaustiveness in that case relies entirely
			// on having a catch-all, which we detect inline below.
			_ => Vec::new(),
		};

		let mut covered = std::collections::HashSet::new();

		for case in cases {
			match &case.pattern.kind {
				PatternKind::Underscore => return,

				PatternKind::Identifier(ident) => {
					// A bare ident either names a nullary variant of the subject enum
					// (covers just that variant) or is a binding (catch-all).
					let is_nullary_variant = match subject_ty {
						Type::Enum(enum_name, _) => self
							.find_variant_in_enum(enum_name, &ident.name)
							.map_or(false, |p| p.is_empty()),
						_ => false,
					};
					if is_nullary_variant {
						covered.insert(ident.name.clone());
					} else {
						return;
					}
				}

				PatternKind::Constructor(name, args) => {
					// Only count the variant as fully covered if every arg is
					// itself a catch-all sub-pattern (recursively — a tuple
					// payload like `some (x, y)` counts iff the tuple itself
					// is all-binding). A literal or nested constructor pulls
					// in just a slice of the value space, so we skip.
					let all_catch = args.iter().all(|arg| self.pattern_is_catch_all(arg));
					if all_catch {
						covered.insert(name.name.clone());
					}
				}

				PatternKind::Literal(lit) => {
					if matches!(subject_ty, Type::Bool) {
						if let LiteralKind::Bool(b) = &lit.kind {
							covered.insert(if *b { "true".into() } else { "false".into() });
						}
					}
				}

				_ => {}
			}
		}

		let missing: Vec<String> = required
			.into_iter()
			.filter(|v| !covered.contains(v))
			.collect();

		if !missing.is_empty() {
			self.error(range, WhenNotExhaustive { missing });
		}
	}

	// Exhaustiveness for `when` over `list a`. The value space is split in
	// two: the empty list, and any non-empty list. A `when` is exhaustive
	// iff both halves are covered (or there's an outer catch-all).
	//
	// What counts:
	// - `_` or a bare ident (non-variant): covers everything.
	// - `[]` (List { items: [], rest: None }): covers empty.
	// - `[...]` / `[...rest]` (List { items: [], rest: Some }): covers
	//   both halves at once — any-length match.
	// - `[head, ...]` / `[head, ...rest]` where `head` is a catch-all
	//   sub-pattern: covers non-empty.
	//
	// Patterns like `[a]` or `[a, b, ...]` cover only specific lengths; we
	// don't try to combine multiple of those into "everything ≥ 1". A
	// catch-all is required for the remaining cases.
	fn check_when_list_exhaustive(&mut self, cases: &[CaseNode], range: Range) {
		let mut covers_empty = false;
		let mut covers_non_empty = false;

		for case in cases {
			match &case.pattern.kind {
				PatternKind::Underscore => return,
				PatternKind::Identifier(_) => return,
				PatternKind::List { items, rest } => {
					match (items.is_empty(), rest.is_some()) {
						(true, false) => covers_empty = true,
						(true, true) => return, // `[...]` covers everything
						(false, true) => {
							// `[head_0, ..., head_n, ...rest]` covers non-empty
							// only when every required head is itself a catch-all
							// (recursively — `[(a, b), ...]` qualifies because the
							// tuple head is all-binding).
							let all_catch = items.iter().all(|it| self.pattern_is_catch_all(it));
							if all_catch {
								covers_non_empty = true;
							}
						}
						(false, false) => {}
					}
				}
				_ => {}
			}
		}

		let mut missing = Vec::new();
		if !covers_empty {
			missing.push("[]".into());
		}
		if !covers_non_empty {
			missing.push("[_, ...]".into());
		}
		if !missing.is_empty() {
			self.error(range, WhenNotExhaustive { missing });
		}
	}

	// Exhaustiveness for `when` over a record-typed subject. Records have a
	// single value shape (whatever the type says), so one record pattern
	// whose sub-patterns are all catch-alls (binding identifier or `_`)
	// covers everything — `when r is {a: n, ...rest} { ... }` doesn't need
	// `else`. A pattern with a literal or constructor sub-pattern can
	// fail and isn't enough on its own.
	fn check_when_record_exhaustive(&mut self, cases: &[CaseNode], range: Range) {
		for case in cases {
			match &case.pattern.kind {
				PatternKind::Underscore => return,
				// Bare identifier binds the whole subject. Record subjects
				// don't have nullary-variant ambiguity (those only apply
				// for enum subjects), so this is always a catch-all here.
				PatternKind::Identifier(_) => return,
				PatternKind::Record { fields, .. } => {
					// All listed-field sub-patterns must themselves be
					// catch-alls (recursively — `{point: (x, y), ...}` covers
					// the whole record because the tuple field is all-binding).
					// The `rest` part (if any) carries no failure condition.
					let all_catch = fields.iter().all(|(_, sub)| self.pattern_is_catch_all(sub));
					if all_catch {
						return;
					}
				}
				_ => {}
			}
		}
		self.error(
			range,
			WhenNotExhaustive {
				missing: vec!["else".into()],
			},
		);
	}

	fn find_variant_in_enum(&self, enum_name: &str, variant_name: &str) -> Option<Vec<Type>> {
		self
			.enum_defs
			.get(enum_name)?
			.variants
			.iter()
			.find(|(n, _)| n == variant_name)
			.map(|(_, params)| params.clone())
	}

	fn find_variant_globally(&self, name: &str) -> Vec<(String, Vec<Type>)> {
		let mut results = Vec::new();
		for (enum_name, enum_def) in &self.enum_defs {
			for (variant_name, params) in &enum_def.variants {
				if variant_name == name {
					results.push((enum_name.clone(), params.clone()));
				}
			}
		}
		results
	}

	// A "catch-all" pattern covers every value of its statically-known type —
	// the analyzer uses this to decide whether a `when` arm is exhaustive on
	// its own (no `else` needed). Composed patterns (tuple/record/list/
	// constructor) need to recurse: `some (x, y)` covers `some` only if the
	// inner tuple covers every payload, which it does iff both subs are
	// themselves catch-alls.
	//
	// Special cases:
	// - A bare identifier matching a known nullary variant only covers that
	//   variant, not its enum — so we treat it as non-catch-all.
	// - A list pattern is a catch-all only when it's a pure rest (`[...]` or
	//   `[...rest]`): any length, no required elements.
	// - Constructor patterns only ever cover their named variant, never the
	//   whole enum.
	fn pattern_is_catch_all(&self, pattern: &PatternNode) -> bool {
		match &pattern.kind {
			PatternKind::Underscore => true,
			PatternKind::Identifier(ident) => self
				.find_variant_globally(&ident.name)
				.iter()
				.all(|(_, p)| !p.is_empty()),
			PatternKind::Tuple(entries) => entries.iter().all(|e| self.pattern_is_catch_all(e)),
			PatternKind::Record { fields, .. } => {
				fields.iter().all(|(_, p)| self.pattern_is_catch_all(p))
			}
			PatternKind::List { items, rest } => items.is_empty() && rest.is_some(),
			PatternKind::Constructor(..) | PatternKind::Literal(..) | PatternKind::Interpolation(..) => {
				false
			}
		}
	}

	// Resolve a variant name in pattern position. Uses the subject type to
	// disambiguate when known; otherwise falls back to a global lookup and
	// reports an ambiguity error if more than one enum matches.
	fn resolve_variant_pattern(
		&mut self,
		name: &IdentifierNode,
		subject_ty: &Type,
		nullary_only: bool,
	) -> VariantResolution {
		match subject_ty {
			Type::Enum(enum_name, _) => match self.find_variant_in_enum(enum_name, &name.name) {
				Some(params) => {
					if nullary_only && !params.is_empty() {
						return VariantResolution::NotFound;
					}
					VariantResolution::Found(enum_name.clone(), params)
				}
				None => VariantResolution::NotFound,
			},

			_ => {
				let mut candidates = self.find_variant_globally(&name.name);
				if nullary_only {
					candidates.retain(|(_, p)| p.is_empty());
				}
				if candidates.is_empty() {
					return VariantResolution::NotFound;
				}
				// Local-module enums shadow imported/prelude ones (mirrors
				// `disambiguate_variant_matches` in expression position).
				if candidates.len() > 1 {
					if let Some(module_name) = &self.module_name {
						let prefix = format!("{}.", module_name);
						let local: Vec<_> = candidates
							.iter()
							.filter(|(q, _)| q.starts_with(&prefix))
							.cloned()
							.collect();
						if local.len() == 1 {
							candidates = local;
						}
					}
				}
				match candidates.len() {
					1 => {
						let (enum_name, params) = candidates.into_iter().next().unwrap();
						VariantResolution::Found(enum_name, params)
					}
					_ => {
						let mut enums: Vec<String> = candidates
							.into_iter()
							.map(|(n, _)| {
								// Display the bare enum name; the qualifier is
								// internal-only and would be redundant when both
								// candidates share the same defining module.
								n.rsplit_once('.').map(|(_, b)| b.to_string()).unwrap_or(n)
							})
							.collect();
						enums.sort();
						self.error(
							name.range,
							AmbiguousVariant {
								name: name.name.clone(),
								enums,
							},
						);
						VariantResolution::Ambiguous
					}
				}
			}
		}
	}

	fn unify(&mut self, constraints: &[Constraint]) -> Substitution {
		// split eq constraints out from others, so we can handle them in two passes
		let mut eq_constraints = Vec::new();
		let mut other_constraints = Vec::new();
		for constraint in constraints {
			if let Constraint::Eq(..) = constraint {
				eq_constraints.push(constraint.clone())
			} else {
				other_constraints.push(constraint.clone())
			}
		}

		// first pass handles eq constraints
		let subst1 = self.unify_eq_constraints(&eq_constraints);
		let other_constraints = subst1.apply_to_constraints(&other_constraints);

		// next pass handles gen/inst constraints
		let subst2 = self.unify_gen_inst_constraints(&other_constraints);

		subst1.compose(subst2)
	}

	// Solve a batch of `Eq` constraints into a most-general unifier.
	//
	// Implemented as a worklist over a *mutable, chained* substitution
	// (union-find style) rather than the textbook substitution-passing
	// recursion. The old approach re-applied each new binding to the entire
	// remaining constraint list — and re-cloned a growing substitution map —
	// at every step, which is O(n²) in the number of constraints and
	// dominated analysis time on real modules. Here we instead:
	//   * keep `bindings` (var → type) and `rows` (row var → fields) maps,
	//   * resolve only the *head* of a type through the chain when we need to
	//     inspect it (`resolve_head`), binding lazily,
	//   * push the children of a structural match back onto the worklist,
	//   * normalize the chained maps into an idempotent `Substitution` once at
	//     the end (`deep_resolve`), so every downstream consumer
	//     (`apply_to_type`, discharge, annotate) sees the same shape as before.
	//
	// Processing is depth-first (stack + reversed children), so row-variable
	// allocation order — and therefore error order and inferred var ids —
	// matches the previous recursive solver.
	fn unify_eq_constraints(&mut self, constraints: &[Constraint]) -> Substitution {
		use Constraint::*;

		let mut bindings: HashMap<usize, Type> = HashMap::new();
		let mut rows: HashMap<usize, RowSolution> = HashMap::new();
		let mut tuple_rows: HashMap<usize, TupleRowSolution> = HashMap::new();

		// Worklist of (lhs, rhs, range). Seeded in reverse so the first
		// constraint is processed first (LIFO + reversed = original order).
		let mut work: Vec<(Type, Type, Range)> = Vec::with_capacity(constraints.len());
		for c in constraints.iter().rev() {
			match c {
				Eq(a, b, reason) => work.push((a.clone(), b.clone(), reason.range)),
				_ => unreachable!("should only have eq constraints in here"),
			}
		}
		// A collapsed range matches what the old solver attached to most
		// structural children (only Fun children and record fields carried the
		// outer range). Preserved so nested error reporting is unchanged.
		let inner = Range::collapsed(0, 0);

		while let Some((a, b, range)) = work.pop() {
			let a = Self::resolve_head(&bindings, a);
			let b = Self::resolve_head(&bindings, b);

			// Match by value: `a`/`b` are owned here, so structural children are
			// *moved* onto the worklist and a bound type is *moved* into the map.
			// (The previous `match (&a, &b)` cloned every child and every bound
			// type; cloning — not hashing — is the dominant cost of this loop.)
			match (a, b) {
				// Leaf types: equal iff identical; nothing to bind.
				(Type::Int, Type::Int)
				| (Type::Float, Type::Float)
				| (Type::Bool, Type::Bool)
				| (Type::String, Type::String)
				| (Type::Bytes, Type::Bytes)
				| (Type::Regex, Type::Regex)
				| (Type::Instant, Type::Instant)
				| (Type::Duration, Type::Duration)
				| (Type::Nothing, Type::Nothing)
				| (Type::Unknown, Type::Unknown) => {}

				// Two identical (unbound) type vars: already equal.
				(Type::Var(n), Type::Var(m)) if n == m => {}

				// A type var on either side binds to the other. `a` (the var on
				// the left) is checked first, matching the old left-biased
				// binding; `(_, Var)` is the `a`-is-not-a-var case.
				(Type::Var(n), b) => {
					if Self::occurs_in(&bindings, n, &b) {
						let ty = Self::deep_resolve(&bindings, &rows, &tuple_rows, &b);
						self.error(range, RecursiveUnification { ty });
					} else {
						bindings.insert(n, b);
					}
				}
				(a, Type::Var(m)) => {
					if Self::occurs_in(&bindings, m, &a) {
						let ty = Self::deep_resolve(&bindings, &rows, &tuple_rows, &a);
						self.error(range, RecursiveUnification { ty });
					} else {
						bindings.insert(m, a);
					}
				}

				(Type::Fun(p1, r1), Type::Fun(p2, r2)) => {
					if p1.len() != p2.len() {
						self.error(
							range,
							ParamCountMismatch {
								expected: p2.len(),
								found: p1.len(),
							},
						);
						continue;
					}
					// Fun children carried the outer range in the old solver.
					// Push reversed so params resolve left-to-right, return last.
					work.push((*r1, *r2, range));
					for (x, y) in p1.into_iter().zip(p2).rev() {
						work.push((x, y, range));
					}
				}

				(Type::List(x), Type::List(y)) | (Type::Ref(x), Type::Ref(y)) => {
					work.push((*x, *y, inner));
				}

				(Type::Dict(k1, v1), Type::Dict(k2, v2)) => {
					work.push((*v1, *v2, inner));
					work.push((*k1, *k2, inner));
				}

				(Type::Tuple(e1), Type::Tuple(e2)) => {
					if e1.len() != e2.len() {
						self.error(
							range,
							TupleSizeMismatch {
								expected: e2.len(),
								found: e1.len(),
							},
						);
						continue;
					}
					for (x, y) in e1.into_iter().zip(e2).rev() {
						work.push((x, y, inner));
					}
				}

				(a @ Type::Tuple(..), b @ Type::PartialTuple(..))
				| (a @ Type::PartialTuple(..), b @ Type::Tuple(..)) => {
					// Resolve the partial tuple fully so its tail's indices are
					// inlined, then check each known index against the concrete
					// tuple and unify element-wise.
					let ra = Self::deep_resolve(&bindings, &rows, &tuple_rows, &a);
					let rb = Self::deep_resolve(&bindings, &rows, &tuple_rows, &b);
					let (elements, fields, tail) = match (ra, rb) {
						(Type::Tuple(e), Type::PartialTuple(f, t))
						| (Type::PartialTuple(f, t), Type::Tuple(e)) => (e, f, t),
						_ => unreachable!(),
					};
					let known: std::collections::HashSet<usize> = fields.iter().map(|(i, _)| *i).collect();
					let mut out_of_bounds = false;
					for (i, t) in fields {
						if i >= elements.len() {
							out_of_bounds = true;
							self.error(
								range,
								TupleIndexNotPresent {
									index: i,
									ty: Type::Tuple(elements.clone()),
								},
							);
							continue;
						}
						work.push((elements[i].clone(), t, inner));
					}
					// Close the partial tuple's tail with the remaining indices —
					// the concrete tuple pins down exactly which indices exist,
					// mirroring how a closed record closes an open record's row var.
					if let (Some(r), false) = (tail, out_of_bounds) {
						let remaining: Vec<(usize, Type)> = (0..elements.len())
							.filter(|i| !known.contains(i))
							.map(|i| (i, elements[i].clone()))
							.collect();
						tuple_rows.insert(
							r,
							TupleRowSolution {
								fields: remaining,
								tail: None,
							},
						);
					}
				}

				(a @ Type::PartialTuple(..), b @ Type::PartialTuple(..)) => {
					// Both partial — resolve fully (inline tail indices) and merge
					// through the tuple row machinery, exactly like Record/Record.
					let (f1, t1) = match Self::deep_resolve(&bindings, &rows, &tuple_rows, &a) {
						Type::PartialTuple(f, t) => (f, t),
						_ => unreachable!(),
					};
					let (f2, t2) = match Self::deep_resolve(&bindings, &rows, &tuple_rows, &b) {
						Type::PartialTuple(f, t) => (f, t),
						_ => unreachable!(),
					};
					self.unify_tuples_worklist(&f1, t1, &f2, t2, range, &mut work, &mut tuple_rows);
				}

				(a @ Type::Record(..), b @ Type::Record(..)) => {
					// Fully resolve both records (inline known row fields,
					// resolve field type heads) before matching, mirroring the
					// substitution the old solver had already applied.
					let (f1, t1) = match Self::deep_resolve(&bindings, &rows, &tuple_rows, &a) {
						Type::Record(f, t) => (f, t),
						_ => unreachable!(),
					};
					let (f2, t2) = match Self::deep_resolve(&bindings, &rows, &tuple_rows, &b) {
						Type::Record(f, t) => (f, t),
						_ => unreachable!(),
					};
					self.unify_records_worklist(&f1, t1, &f2, t2, range, &mut work, &mut rows);
				}

				(Type::Enum(n1, args1), Type::Enum(n2, args2)) if n1 == n2 => {
					// Names match → unify the type-arg lists pairwise. An arity
					// mismatch here is an internal bug (caught upstream).
					debug_assert_eq!(args1.len(), args2.len());
					for (x, y) in args1.into_iter().zip(args2).rev() {
						work.push((x, y, inner));
					}
				}

				// Anything else is a genuine mismatch.
				(a, b) => {
					let expected = Self::deep_resolve(&bindings, &rows, &tuple_rows, &b);
					let found = Self::deep_resolve(&bindings, &rows, &tuple_rows, &a);
					self.error(range, TypeMismatch { expected, found });
				}
			}
		}

		// Normalize the chained maps into an idempotent substitution — the
		// shape `Substitution::apply_to_type` expects (single-level var lookup,
		// transitive row-tail chasing).
		let mut solutions: HashMap<usize, Type> = HashMap::with_capacity(bindings.len());
		for (k, v) in &bindings {
			solutions.insert(*k, Self::deep_resolve(&bindings, &rows, &tuple_rows, v));
		}
		let mut row_solutions: HashMap<usize, RowSolution> = HashMap::with_capacity(rows.len());
		for (k, sol) in &rows {
			row_solutions.insert(
				*k,
				RowSolution {
					fields: sol
						.fields
						.iter()
						.map(|(n, t)| {
							(
								n.clone(),
								Self::deep_resolve(&bindings, &rows, &tuple_rows, t),
							)
						})
						.collect(),
					tail: sol.tail,
				},
			);
		}
		let mut tuple_row_solutions: HashMap<usize, TupleRowSolution> =
			HashMap::with_capacity(tuple_rows.len());
		for (k, sol) in &tuple_rows {
			tuple_row_solutions.insert(
				*k,
				TupleRowSolution {
					fields: sol
						.fields
						.iter()
						.map(|(i, t)| (*i, Self::deep_resolve(&bindings, &rows, &tuple_rows, t)))
						.collect(),
					tail: sol.tail,
				},
			);
		}
		Substitution {
			solutions,
			row_solutions,
			tuple_row_solutions,
		}
	}

	// Follow a chain of variable bindings at the *head* of a type only.
	// Returns the first non-variable type, or an unbound variable.
	fn resolve_head(bindings: &HashMap<usize, Type>, ty: Type) -> Type {
		let mut cur = ty;
		while let Type::Var(n) = cur {
			match bindings.get(&n) {
				Some(next) => cur = next.clone(),
				None => return Type::Var(n),
			}
		}
		cur
	}

	// Does `var` occur anywhere in `ty`, resolving variables through the
	// current bindings? Mirrors the old (post-substitution) `contains_var`
	// occurs check, including ignoring record tails.
	fn occurs_in(bindings: &HashMap<usize, Type>, var: usize, ty: &Type) -> bool {
		// Resolve the head through the binding chain *by reference* — no cloning.
		// (The old version cloned the whole subtree just to inspect its head,
		// at every recursion level.)
		let mut head: &Type = ty;
		loop {
			match head {
				Type::Var(n) => match bindings.get(n) {
					Some(next) => head = next,
					None => return *n == var,
				},
				_ => break,
			}
		}
		match head {
			Type::Fun(params, ret) => {
				params.iter().any(|p| Self::occurs_in(bindings, var, p))
					|| Self::occurs_in(bindings, var, ret)
			}
			Type::List(e) | Type::Ref(e) => Self::occurs_in(bindings, var, e),
			Type::Dict(k, v) => Self::occurs_in(bindings, var, k) || Self::occurs_in(bindings, var, v),
			Type::Tuple(es) | Type::Enum(_, es) => es.iter().any(|e| Self::occurs_in(bindings, var, e)),
			Type::PartialTuple(fields, _) => fields
				.iter()
				.any(|(_, t)| Self::occurs_in(bindings, var, t)),
			Type::Record(fields, _) => fields
				.iter()
				.any(|(_, t)| Self::occurs_in(bindings, var, t)),
			_ => false,
		}
	}

	// Fully resolve a type through the chained bindings + row solutions,
	// producing an idempotent type. Equivalent to applying the final
	// substitution to `ty`; kept in lockstep with `Substitution::apply_to_type`.
	fn deep_resolve(
		bindings: &HashMap<usize, Type>,
		rows: &HashMap<usize, RowSolution>,
		tuple_rows: &HashMap<usize, TupleRowSolution>,
		ty: &Type,
	) -> Type {
		match ty {
			Type::Var(n) => match bindings.get(n) {
				Some(t) => Self::deep_resolve(bindings, rows, tuple_rows, t),
				None => Type::Var(*n),
			},
			Type::Unknown
			| Type::Nothing
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Bytes
			| Type::Regex
			| Type::Instant
			| Type::Duration => ty.clone(),
			Type::Enum(name, args) => Type::Enum(
				name.clone(),
				args
					.iter()
					.map(|t| Self::deep_resolve(bindings, rows, tuple_rows, t))
					.collect(),
			),
			Type::Fun(params, ret) => Type::Fun(
				params
					.iter()
					.map(|t| Self::deep_resolve(bindings, rows, tuple_rows, t))
					.collect(),
				Self::deep_resolve(bindings, rows, tuple_rows, ret).into(),
			),
			Type::PartialTuple(fields, tail) => {
				let mut new_fields: Vec<(usize, Type)> = fields
					.iter()
					.map(|(i, t)| (*i, Self::deep_resolve(bindings, rows, tuple_rows, t)))
					.collect();
				let mut current_tail = *tail;
				while let Some(rid) = current_tail {
					match tuple_rows.get(&rid) {
						Some(sol) => {
							for (i, t) in &sol.fields {
								new_fields.push((*i, Self::deep_resolve(bindings, rows, tuple_rows, t)));
							}
							current_tail = sol.tail;
						}
						None => break,
					}
				}
				Type::PartialTuple(new_fields, current_tail)
			}
			Type::Tuple(elements) => Type::Tuple(
				elements
					.iter()
					.map(|t| Self::deep_resolve(bindings, rows, tuple_rows, t))
					.collect(),
			),
			Type::List(e) => Type::List(Self::deep_resolve(bindings, rows, tuple_rows, e).into()),
			Type::Dict(k, v) => Type::Dict(
				Self::deep_resolve(bindings, rows, tuple_rows, k).into(),
				Self::deep_resolve(bindings, rows, tuple_rows, v).into(),
			),
			Type::Ref(inner) => Type::Ref(Self::deep_resolve(bindings, rows, tuple_rows, inner).into()),
			Type::Record(fields, tail) => {
				let mut new_fields: Vec<(String, Type)> = fields
					.iter()
					.map(|(n, t)| (n.clone(), Self::deep_resolve(bindings, rows, tuple_rows, t)))
					.collect();
				let mut current_tail = *tail;
				while let Some(rid) = current_tail {
					match rows.get(&rid) {
						Some(sol) => {
							for (n, t) in &sol.fields {
								new_fields.push((n.clone(), Self::deep_resolve(bindings, rows, tuple_rows, t)));
							}
							current_tail = sol.tail;
						}
						None => break,
					}
				}
				Type::Record(new_fields, current_tail)
			}
		}
	}

	// Row-polymorphic record unification, worklist edition. Same four cases as
	// the old `unify_records`, but pushes shared-field constraints onto `work`
	// and inserts row solutions into `rows` instead of building/composing a
	// fresh `Substitution`. Both records are already fully resolved by the
	// caller, so their tails are `None` or an unbound row var.
	//
	//   (None, None)         — both closed: field sets must be equal
	//   (None, Some(r))      — r absorbs the fields only the closed side has
	//   (Some(r), None)      — symmetric
	//   (Some(r1), Some(r2)) — fresh row var t; r1 := only_2 + t, r2 := only_1 + t
	fn unify_records_worklist(
		&mut self,
		fields_1: &[(String, Type)],
		tail_1: Option<usize>,
		fields_2: &[(String, Type)],
		tail_2: Option<usize>,
		range: Range,
		work: &mut Vec<(Type, Type, Range)>,
		rows: &mut HashMap<usize, RowSolution>,
	) {
		let map_1: HashMap<&String, usize> = fields_1
			.iter()
			.enumerate()
			.map(|(i, (k, _))| (k, i))
			.collect();
		let map_2: HashMap<&String, usize> = fields_2
			.iter()
			.enumerate()
			.map(|(i, (k, _))| (k, i))
			.collect();

		// Common fields → unify pairwise (carry the outer range, as the old
		// solver did for shared fields).
		let mut shared: Vec<(Type, Type)> = Vec::new();
		for (name, i1) in &map_1 {
			if let Some(i2) = map_2.get(*name) {
				shared.push((fields_1[*i1].1.clone(), fields_2[*i2].1.clone()));
			}
		}

		// Fields unique to each side (order-preserving for stable snapshots).
		let only_1: Vec<(String, Type)> = fields_1
			.iter()
			.filter(|(n, _)| !map_2.contains_key(n))
			.cloned()
			.collect();
		let only_2: Vec<(String, Type)> = fields_2
			.iter()
			.filter(|(n, _)| !map_1.contains_key(n))
			.cloned()
			.collect();

		let push_shared = |work: &mut Vec<(Type, Type, Range)>| {
			for (a, b) in &shared {
				work.push((a.clone(), b.clone(), range));
			}
		};

		match (tail_1, tail_2) {
			(None, None) => {
				let mut ok = true;
				for (n, _) in &only_1 {
					ok = false;
					self.error(
						range,
						RecordFieldNotPresent {
							field: n.clone(),
							ty: Type::Record(fields_2.to_vec(), None),
						},
					);
				}
				for (n, _) in &only_2 {
					ok = false;
					self.error(
						range,
						RecordFieldNotPresent {
							field: n.clone(),
							ty: Type::Record(fields_1.to_vec(), None),
						},
					);
				}
				if ok {
					push_shared(work);
				}
			}

			(None, Some(r2)) => {
				// Left is closed; right's row var absorbs left's extras. Right's
				// listed-but-not-on-left fields can't exist on a closed left.
				let mut ok = true;
				for (n, _) in &only_2 {
					ok = false;
					self.error(
						range,
						RecordFieldNotPresent {
							field: n.clone(),
							ty: Type::Record(fields_1.to_vec(), None),
						},
					);
				}
				if ok {
					push_shared(work);
					rows.insert(
						r2,
						RowSolution {
							fields: only_1,
							tail: None,
						},
					);
				}
			}

			(Some(r1), None) => {
				let mut ok = true;
				for (n, _) in &only_1 {
					ok = false;
					self.error(
						range,
						RecordFieldNotPresent {
							field: n.clone(),
							ty: Type::Record(fields_2.to_vec(), None),
						},
					);
				}
				if ok {
					push_shared(work);
					rows.insert(
						r1,
						RowSolution {
							fields: only_2,
							tail: None,
						},
					);
				}
			}

			(Some(r1), Some(r2)) => {
				if r1 == r2 {
					// Same row var on both sides: consistent only if neither
					// side claims unique fields.
					if !only_1.is_empty() || !only_2.is_empty() {
						self.error(
							range,
							TypeMismatch {
								expected: Type::Record(fields_2.to_vec(), Some(r2)),
								found: Type::Record(fields_1.to_vec(), Some(r1)),
							},
						);
						return;
					}
					push_shared(work);
					return;
				}
				// Fresh row var captures the unknown tail shared by both sides.
				let fresh = self.new_row_var();
				push_shared(work);
				rows.insert(
					r1,
					RowSolution {
						fields: only_2,
						tail: Some(fresh),
					},
				);
				rows.insert(
					r2,
					RowSolution {
						fields: only_1,
						tail: Some(fresh),
					},
				);
			}
		}
	}

	// The tuple analogue of `unify_records_worklist`: same four cases, keyed
	// by tuple index instead of field name. Both partial tuples are already
	// fully resolved by the caller, so their tails are `None` or an unbound
	// row var. A unique index on a closed side is an outright mismatch (a
	// closed tuple can't grow), reported by reconstructing the two shapes.
	fn unify_tuples_worklist(
		&mut self,
		fields_1: &[(usize, Type)],
		tail_1: Option<usize>,
		fields_2: &[(usize, Type)],
		tail_2: Option<usize>,
		range: Range,
		work: &mut Vec<(Type, Type, Range)>,
		tuple_rows: &mut HashMap<usize, TupleRowSolution>,
	) {
		let map_1: HashMap<usize, usize> = fields_1
			.iter()
			.enumerate()
			.map(|(i, (k, _))| (*k, i))
			.collect();
		let map_2: HashMap<usize, usize> = fields_2
			.iter()
			.enumerate()
			.map(|(i, (k, _))| (*k, i))
			.collect();

		// Indices present on both sides → unify pairwise.
		let mut shared: Vec<(Type, Type)> = Vec::new();
		for (index, i1) in &map_1 {
			if let Some(i2) = map_2.get(index) {
				shared.push((fields_1[*i1].1.clone(), fields_2[*i2].1.clone()));
			}
		}

		// Indices unique to each side (order-preserving for stable snapshots).
		let only_1: Vec<(usize, Type)> = fields_1
			.iter()
			.filter(|(i, _)| !map_2.contains_key(i))
			.cloned()
			.collect();
		let only_2: Vec<(usize, Type)> = fields_2
			.iter()
			.filter(|(i, _)| !map_1.contains_key(i))
			.cloned()
			.collect();

		let push_shared = |work: &mut Vec<(Type, Type, Range)>| {
			for (a, b) in &shared {
				work.push((a.clone(), b.clone(), range));
			}
		};

		// A closed side can't carry indices the other side lacks.
		let mismatch = |me: &mut Self| {
			me.error(
				range,
				TypeMismatch {
					expected: Type::PartialTuple(fields_2.to_vec(), tail_2),
					found: Type::PartialTuple(fields_1.to_vec(), tail_1),
				},
			);
		};

		match (tail_1, tail_2) {
			(None, None) => {
				if !only_1.is_empty() || !only_2.is_empty() {
					mismatch(self);
				} else {
					push_shared(work);
				}
			}

			(None, Some(r2)) => {
				// Left is closed; right's row var absorbs left's extra indices.
				if !only_2.is_empty() {
					mismatch(self);
				} else {
					push_shared(work);
					tuple_rows.insert(
						r2,
						TupleRowSolution {
							fields: only_1,
							tail: None,
						},
					);
				}
			}

			(Some(r1), None) => {
				if !only_1.is_empty() {
					mismatch(self);
				} else {
					push_shared(work);
					tuple_rows.insert(
						r1,
						TupleRowSolution {
							fields: only_2,
							tail: None,
						},
					);
				}
			}

			(Some(r1), Some(r2)) => {
				if r1 == r2 {
					// Same row var on both sides: consistent only if neither
					// side claims unique indices.
					if !only_1.is_empty() || !only_2.is_empty() {
						mismatch(self);
						return;
					}
					push_shared(work);
					return;
				}
				// Fresh row var captures the tail shared by both sides.
				let fresh = self.new_row_var();
				push_shared(work);
				tuple_rows.insert(
					r1,
					TupleRowSolution {
						fields: only_2,
						tail: Some(fresh),
					},
				);
				tuple_rows.insert(
					r2,
					TupleRowSolution {
						fields: only_1,
						tail: Some(fresh),
					},
				);
			}
		}
	}

	fn unify_gen_inst_constraints(&mut self, constraints: &[Constraint]) -> Substitution {
		if constraints.is_empty() {
			return Substitution::empty();
		}

		// Find any Gen to process. Self-recursive defs produce Insts (from the
		// recursive lookup) before the Gen (which is pushed after the body is
		// constrained), so we can't assume constraints[0] is the Gen.
		// If only Class (and other non-Gen) constraints remain, we're done at
		// this level — those get picked up by discharge after unify returns.
		let gen_idx = match constraints
			.iter()
			.position(|c| matches!(c, Constraint::Gen(..)))
		{
			Some(idx) => idx,
			None => return Substitution::empty(),
		};

		match &constraints[gen_idx] {
			Constraint::Gen(scheme, ty) => {
				let mut inst_constraints_for_gen = Vec::new();
				let mut class_pool: Vec<ClassConstraint> = Vec::new();
				let mut other_constraints = Vec::new();
				for (i, constraint) in constraints.iter().enumerate() {
					if i == gen_idx {
						continue;
					}
					match constraint {
						Constraint::Inst(var1, ..) => match scheme {
							Scheme::Var(var2) if *var1 == *var2 => {
								inst_constraints_for_gen.push(constraint.clone());
							}
							_ => other_constraints.push(constraint.clone()),
						},
						Constraint::Class(c) => {
							class_pool.push(c.clone());
							other_constraints.push(constraint.clone());
						}
						_ => other_constraints.push(constraint.clone()),
					}
				}

				// For each Inst against this scheme, instantiate the scheme
				// (Eq + fresh Class constraints) and push fresh dispatch
				// cells into the originating Call's sink. We feed the new
				// constraints back into unify so any class constraints they
				// generate flow with the rest.
				let new_constraints =
					self.instantiate_constraints(&inst_constraints_for_gen, &ty, &class_pool);
				// Split: Eq go to subst solving immediately; new Class
				// constraints get appended to other_constraints so they
				// survive into discharge.
				let mut new_eq = Vec::new();
				for c in new_constraints {
					match c {
						Constraint::Eq(..) => new_eq.push(c),
						Constraint::Class(_) => other_constraints.push(c),
						_ => unreachable!("instantiate_constraints only emits Eq and Class"),
					}
				}

				let subst = self.unify_eq_constraints(&new_eq);
				let other_constraints = subst.apply_to_constraints(&other_constraints);
				let subst2 = self.unify_gen_inst_constraints(&other_constraints);

				subst.compose(subst2)
			}

			_ => unreachable!(),
		}
	}

	fn fill_in_placeholder(&mut self, ty: &mut Type, subst: &Substitution) {
		if let Type::Var(n) = ty {
			if let Some(actual_type) = subst.solutions.get(&n) {
				*ty = actual_type.clone();
			}
		}
	}

	// Walk the class constraint set after unification. For each
	// `Class name ty`:
	//   - concrete `ty` + matching instance → write `Resolved::Global(slot)`
	//     into the shared dispatch cell so codegen knows which dict to load.
	//   - concrete `ty` + no instance → diagnostic.
	//   - `ty` still a type var → leave the constraint alone. Generalization
	//     will push it into the surrounding scheme, or (if it escapes any
	//     enclosing def boundary) phase 4 will flag the ambiguity.
	fn discharge(&mut self, class_constraints: &[ClassConstraint]) {
		for c in class_constraints {
			// Skip unresolved tyvar dispatches — those flow into
			// generalization and become part of the enclosing scheme.
			if matches!(c.ty, Type::Var(_)) {
				continue;
			}

			match self.try_resolve_dispatch(&c.name, &c.ty) {
				Some(resolved) => {
					c.dispatch_cell.borrow_mut().resolved = Some(resolved);
				}
				None => {
					// `wire` is auto-derived, so a failure means the type isn't
					// serializable — explain with attribution (FULLSTACK.md),
					// unless the only obstacle is a free var (→ ambiguity below).
					let wire_detail = if c.name == "wire" {
						self.wire_underivable_detail(&c.ty, &mut Vec::new())
					} else {
						None
					};
					let kind = if let Some(detail) = wire_detail {
						AnalysisErrorKind::NotWireDerivable {
							ty: c.ty.clone(),
							detail,
						}
					} else if !c.ty.free_vars().is_empty() {
						// A free type variable means ambiguity, not a missing
						// instance — e.g. `showable.show none` where `none :
						// option ?`. Tell the user to annotate.
						AnalysisErrorKind::AmbiguousTraitMethod {
							trait_name: c.name.clone(),
							ty: c.ty.clone(),
						}
					} else {
						AnalysisErrorKind::NoInstance {
							trait_name: c.name.clone(),
							ty: c.ty.clone(),
						}
					};
					self.error(c.reason.range, kind);
				}
			}
		}
	}

	// Recursively resolve a `(trait, ty)` dispatch to a `Resolved`. Returns
	// `None` if no instance matches. Concrete instances → `Global`.
	// Parametric instances → `InstanceChain` with each `where`-clause
	// constraint resolved against the unifying substitution.
	fn try_resolve_dispatch(&self, trait_name: &str, ty: &Type) -> Option<Resolved> {
		// `wire` is auto-derived: there are no registered instances. Resolve it
		// by synthesizing a schema shape from `ty`'s structure (FULLSTACK.md).
		// `None` means non-derivable → discharge reports it as a missing
		// instance (attribution refined in M4).
		if trait_name == "wire" {
			return self
				.build_wire_shape(ty, &mut Vec::new())
				.map(Resolved::WireSchema);
		}

		let head_key = type_to_head_key(ty)?;
		let inst = self.instances.get(&(trait_name.to_string(), head_key))?;

		if inst.param_vars.is_empty() {
			// Concrete instance — must match `ty` exactly.
			if !type_keys_match(&inst.head_type, ty) {
				return None;
			}
			return Some(Resolved::Global(inst.instance_slot_name.clone()));
		}

		// Parametric: match the instance head against `ty` to bind the
		// instance's param tyvars to concrete subterms, then recursively
		// resolve each `where`-clause constraint.
		let mut mapping: std::collections::HashMap<usize, Type> = std::collections::HashMap::new();
		if !match_types(&inst.head_type, ty, &mut mapping) {
			return None;
		}

		let mut inner: Vec<Resolved> = Vec::new();
		for (wc_trait, wc_var) in &inst.where_clauses {
			let wc_ty = mapping.get(wc_var).cloned()?;
			let inner_resolved = self.try_resolve_dispatch(wc_trait, &wc_ty)?;
			inner.push(inner_resolved);
		}

		Some(Resolved::InstanceChain {
			ctor_slot: inst.instance_slot_name.clone(),
			inner,
		})
	}

	// Walk the AST and rewrite each `try` expression whose RHS head
	// constructor is resolved into the equivalent `<carrier>.then` call,
	// emitting linking constraints into `new_constraints`. `try`s whose
	// RHS is still an unresolved tyvar are LEFT IN PLACE so a subsequent
	// iteration (after re-unifying against the new constraints) can take
	// another pass. Returns `true` if at least one node was rewritten.
	fn dispatch_try_nodes(
		&mut self,
		module: &mut ModuleNode,
		subst: &Substitution,
		new_constraints: &mut Vec<Constraint>,
	) -> bool {
		let mut dispatched_any = false;
		for definition in &mut module.body {
			match &mut definition.kind {
				DefinitionKind::Expr(expr) => {
					self.dispatch_try_in_expr(expr, subst, new_constraints, &mut dispatched_any, None);
				}
				DefinitionKind::Instance(instance_node) => {
					for method in &mut instance_node.methods {
						if let DefinitionKind::Expr(expr) = &mut method.kind {
							self.dispatch_try_in_expr(expr, subst, new_constraints, &mut dispatched_any, None);
						}
					}
				}
				_ => {}
			}
		}
		dispatched_any
	}

	// Walk the AST after the dispatch fixpoint and emit diagnostics for
	// any `try` nodes that never got resolved (their RHS type stayed an
	// unbound tyvar). Each remaining node also has its expr.ty set to
	// `Type::Unknown` so codegen doesn't trip over it (codegen errors on
	// any `try` node it sees, since the contract is "analyzer rewrites
	// every try").
	fn report_unresolved_try_nodes(&mut self, module: &mut ModuleNode, subst: &Substitution) {
		for definition in &mut module.body {
			match &mut definition.kind {
				DefinitionKind::Expr(expr) => {
					self.report_unresolved_try_in_expr(expr, subst);
				}
				DefinitionKind::Instance(instance_node) => {
					for method in &mut instance_node.methods {
						if let DefinitionKind::Expr(expr) = &mut method.kind {
							self.report_unresolved_try_in_expr(expr, subst);
						}
					}
				}
				_ => {}
			}
		}
	}

	fn report_unresolved_try_in_expr(&mut self, expr: &mut ExprNode, subst: &Substitution) {
		match &mut expr.kind {
			ExprKind::Fun(FunNode { body, .. }) => {
				for e in body.iter_mut() {
					self.report_unresolved_try_in_expr(e, subst);
				}
			}
			ExprKind::Call(CallNode { callee, args, .. }) => {
				self.report_unresolved_try_in_expr(callee, subst);
				for a in args.iter_mut() {
					self.report_unresolved_try_in_expr(a, subst);
				}
			}
			ExprKind::Let(LetNode { value, .. }) => {
				self.report_unresolved_try_in_expr(value, subst);
			}
			ExprKind::Tuple(es) | ExprKind::Interpolation(es) => {
				for e in es.iter_mut() {
					self.report_unresolved_try_in_expr(e, subst);
				}
			}
			ExprKind::List(items) => {
				for item in items.iter_mut() {
					self.report_unresolved_try_in_expr(item.expr_mut(), subst);
				}
			}
			ExprKind::Record(fields) => {
				for (_, v) in fields.iter_mut() {
					self.report_unresolved_try_in_expr(v, subst);
				}
			}
			ExprKind::RecordUpdate { base, fields } => {
				self.report_unresolved_try_in_expr(base, subst);
				for (_, v) in fields.iter_mut() {
					self.report_unresolved_try_in_expr(v, subst);
				}
			}
			ExprKind::ElementAccess { receiver, .. } | ExprKind::FieldAccess { receiver, .. } => {
				self.report_unresolved_try_in_expr(receiver, subst);
			}
			ExprKind::UnaryOperation { right, .. } => {
				self.report_unresolved_try_in_expr(right, subst);
			}
			ExprKind::BinaryOperation { left, right, op } => {
				self.report_unresolved_try_in_expr(left, subst);
				self.report_unresolved_try_in_expr(right, subst);
				// A `??` still shaped as a BinaryOperation here was never
				// dispatched ⇒ its LHS type stayed an unbound tyvar.
				if matches!(op.kind, Operator::NullCoalescing) {
					self.error(expr.range, AnalysisErrorKind::CoalesceLhsUndetermined);
					expr.ty = Type::Unknown;
				}
			}
			ExprKind::If(IfNode {
				subject,
				body,
				else_body,
				..
			}) => {
				self.report_unresolved_try_in_expr(subject, subst);
				for e in body.iter_mut() {
					self.report_unresolved_try_in_expr(e, subst);
				}
				if let Some(else_body) = else_body {
					for e in else_body.iter_mut() {
						self.report_unresolved_try_in_expr(e, subst);
					}
				}
			}
			ExprKind::When(WhenNode { subject, cases, .. }) => {
				self.report_unresolved_try_in_expr(subject, subst);
				for c in cases.iter_mut() {
					for e in c.body.iter_mut() {
						self.report_unresolved_try_in_expr(e, subst);
					}
				}
			}
			ExprKind::While(WhileNode { subject, body, .. }) => {
				self.report_unresolved_try_in_expr(subject, subst);
				for e in body.iter_mut() {
					self.report_unresolved_try_in_expr(e, subst);
				}
			}
			ExprKind::Scope(ScopeNode { body, .. }) => {
				for e in body.iter_mut() {
					self.report_unresolved_try_in_expr(e, subst);
				}
			}
			ExprKind::Grouping(inner) => {
				self.report_unresolved_try_in_expr(inner, subst);
			}
			ExprKind::Defer(inner) => {
				self.report_unresolved_try_in_expr(inner, subst);
			}
			ExprKind::Try(TryNode {
				range,
				value,
				rest,
				task_carrier,
				..
			}) => {
				let is_task = *task_carrier;
				let try_range = *range;
				let resolved = subst.apply_to_type(&value.ty);
				// Walk children first in case they have un-rewritten trys
				// of their own.
				self.report_unresolved_try_in_expr(value, subst);
				for e in rest.iter_mut() {
					self.report_unresolved_try_in_expr(e, subst);
				}
				// A task `try` is intentionally left intact (codegen lowers
				// it); it's resolved, not a failure.
				if !is_task {
					match resolved {
						Type::Var(_) => {
							self.error(try_range, AnalysisErrorKind::TryRhsUndetermined);
						}
						_ => {
							// Should be impossible — dispatch loop should have
							// handled any non-Var head. Report as unsupported
							// carrier so the user sees the actual type.
							self.error(
								try_range,
								AnalysisErrorKind::TryUnsupportedCarrier { ty: resolved },
							);
						}
					}
					expr.ty = Type::Unknown;
				}
			}
			ExprKind::Identifier(_)
			| ExprKind::Literal(_)
			| ExprKind::Regex(_)
			| ExprKind::EmptyTuple
			| ExprKind::Builtin(_)
			| ExprKind::NamespaceAccess(_) => {}
		}
	}

	fn dispatch_try_in_expr(
		&mut self,
		expr: &mut ExprNode,
		subst: &Substitution,
		new_constraints: &mut Vec<Constraint>,
		dispatched_any: &mut bool,
		// The tail type of the enclosing async context (a `fun` body), used to
		// enforce that a function which awaits returns a task. `None` at the top
		// level and inside `scope` bodies (which carry their own task constraint).
		enclosing_tail: Option<&Type>,
	) {
		// Walk children first so nested `try`s are rewritten bottom-up.
		// For Try, we recurse into its own children below before rewriting
		// this node.
		match &mut expr.kind {
			ExprKind::Fun(FunNode { body, .. }) => {
				// A `fun` is an async context: a `try` anywhere in its body ties
				// *this* fun's tail to a task. Capture the tail type var before
				// recursing (it stays the same var through the walk).
				let tail = body.last().map(|e| e.ty.clone());
				for e in body.iter_mut() {
					self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, tail.as_ref());
				}
			}
			ExprKind::Call(CallNode { callee, args, .. }) => {
				self.dispatch_try_in_expr(
					callee,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
				for a in args.iter_mut() {
					self.dispatch_try_in_expr(a, subst, new_constraints, dispatched_any, enclosing_tail);
				}
			}
			ExprKind::Let(LetNode { value, .. }) => {
				self.dispatch_try_in_expr(
					value,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
			}
			ExprKind::Tuple(es) | ExprKind::Interpolation(es) => {
				for e in es.iter_mut() {
					self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, enclosing_tail);
				}
			}
			ExprKind::List(items) => {
				for item in items.iter_mut() {
					self.dispatch_try_in_expr(
						item.expr_mut(),
						subst,
						new_constraints,
						dispatched_any,
						enclosing_tail,
					);
				}
			}
			ExprKind::Record(fields) => {
				for (_, v) in fields.iter_mut() {
					self.dispatch_try_in_expr(v, subst, new_constraints, dispatched_any, enclosing_tail);
				}
			}
			ExprKind::RecordUpdate { base, fields } => {
				self.dispatch_try_in_expr(base, subst, new_constraints, dispatched_any, enclosing_tail);
				for (_, v) in fields.iter_mut() {
					self.dispatch_try_in_expr(v, subst, new_constraints, dispatched_any, enclosing_tail);
				}
			}
			ExprKind::ElementAccess { receiver, .. } | ExprKind::FieldAccess { receiver, .. } => {
				self.dispatch_try_in_expr(
					receiver,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
			}
			ExprKind::UnaryOperation { right, .. } => {
				self.dispatch_try_in_expr(
					right,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
			}
			ExprKind::BinaryOperation { left, right, op } => {
				self.dispatch_try_in_expr(left, subst, new_constraints, dispatched_any, enclosing_tail);
				self.dispatch_try_in_expr(
					right,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
				// `??` dispatches like `try`: once the LHS type is pinned, this
				// node is rewritten into a `<carrier>.or-else` call.
				let is_coalesce = matches!(op.kind, Operator::NullCoalescing);
				if is_coalesce && self.do_coalesce_dispatch(expr, subst, new_constraints) {
					*dispatched_any = true;
				}
			}
			ExprKind::If(IfNode {
				subject,
				body,
				else_body,
				..
			}) => {
				self.dispatch_try_in_expr(
					subject,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
				for e in body.iter_mut() {
					self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, enclosing_tail);
				}
				if let Some(else_body) = else_body {
					for e in else_body.iter_mut() {
						self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, enclosing_tail);
					}
				}
			}
			ExprKind::When(WhenNode { subject, cases, .. }) => {
				self.dispatch_try_in_expr(
					subject,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
				for c in cases.iter_mut() {
					for e in c.body.iter_mut() {
						self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, enclosing_tail);
					}
				}
			}
			ExprKind::While(WhileNode { subject, body, .. }) => {
				self.dispatch_try_in_expr(
					subject,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
				for e in body.iter_mut() {
					self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, enclosing_tail);
				}
			}
			ExprKind::Scope(ScopeNode { body, .. }) => {
				// A `scope` body is its own async context (its tail is already
				// constrained to a task where the scope is typed), so a `try` inside
				// it must not tie the *enclosing* fun's tail — pass `None`.
				for e in body.iter_mut() {
					self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, None);
				}
			}
			ExprKind::Grouping(inner) => {
				self.dispatch_try_in_expr(
					inner,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
			}
			ExprKind::Defer(inner) => {
				self.dispatch_try_in_expr(
					inner,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
			}
			ExprKind::Try(TryNode { value, rest, .. }) => {
				// Recurse into THIS try's children first (so nested trys
				// in `value` or `rest` get rewritten before we touch the
				// outer node). Borrow re-acquired after the recursion.
				self.dispatch_try_in_expr(
					value,
					subst,
					new_constraints,
					dispatched_any,
					enclosing_tail,
				);
				for e in rest.iter_mut() {
					self.dispatch_try_in_expr(e, subst, new_constraints, dispatched_any, enclosing_tail);
				}
				if self.do_try_dispatch(expr, subst, new_constraints, enclosing_tail) {
					*dispatched_any = true;
				}
			}
			ExprKind::Identifier(_)
			| ExprKind::Literal(_)
			| ExprKind::Regex(_)
			| ExprKind::EmptyTuple
			| ExprKind::Builtin(_)
			| ExprKind::NamespaceAccess(_) => {}
		}
	}

	// Perform the actual rewrite of one Try expression. `expr.kind` must
	// be `Try(_)` on entry. Returns `true` if the rewrite succeeded (now
	// `Call`), `false` if the RHS type isn't pinned yet (`expr.kind` is
	// left as `Try` for a later iteration). Diagnosable failures (empty
	// body, unsupported pattern, unsupported carrier) report and rewrite
	// to `EmptyTuple` with `Type::Unknown` — still counted as "handled"
	// so the loop terminates.
	fn do_try_dispatch(
		&mut self,
		expr: &mut ExprNode,
		subst: &Substitution,
		new_constraints: &mut Vec<Constraint>,
		enclosing_tail: Option<&Type>,
	) -> bool {
		use AnalysisErrorKind::*;

		// Peek the resolved value type without consuming the Try yet —
		// if it's still a free tyvar, we leave the Try in place so a
		// later iteration (after more constraints get added by other
		// dispatches and re-unify) can revisit it.
		let value_ty_clone = match &expr.kind {
			ExprKind::Try(t) => t.value.ty.clone(),
			_ => unreachable!("do_try_dispatch called on non-Try expr"),
		};
		let resolved_value_ty = subst.apply_to_type(&value_ty_clone);
		if matches!(resolved_value_ty, Type::Var(_)) {
			return false;
		}

		// Task carrier: unlike option/result, we do NOT rewrite the `try`
		// into a `task.then` call. Lowering a task `try`-chain to a tree of
		// `.then` closures *is* the trampoline; for the CPS transform we
		// keep the `Try` node intact (flag it `task_carrier`) so codegen can
		// lay the chain out as one resumable state-machine step function.
		// We only emit the linking constraints, then leave the node for
		// codegen. Idempotent: once flagged, report no further progress so
		// the dispatch fixpoint can terminate.
		if let Type::Enum(name, args) = &resolved_value_ty {
			if name == "__prelude__.task" && args.len() == 1 {
				let payload_ty = args[0].clone();
				let t = match &mut expr.kind {
					ExprKind::Try(t) => t,
					_ => unreachable!("do_try_dispatch called on non-Try expr"),
				};
				if t.task_carrier {
					return false;
				}
				if t.rest.is_empty() {
					self.error(t.range, TryEmptyBody);
					expr.ty = Type::Unknown;
					return true;
				}
				if !matches!(
					t.pattern.kind,
					PatternKind::Identifier(_) | PatternKind::Underscore
				) {
					self.error(t.range, TryUnsupportedPattern);
					expr.ty = Type::Unknown;
					return true;
				}
				let try_range = t.range;
				let pattern_ty = t.pattern_ty.clone();
				let last_idx = t.rest.len() - 1;
				let body_last_ty = t.rest[last_idx].ty.clone();
				t.task_carrier = true;

				// The pattern binds the awaited task's payload (`await` unwraps the
				// `task a` to its `a`). The `try` is *type-transparent* to its
				// continuation — its value is the continuation's value — so it can
				// sit in a statement position (e.g. a `while` body) without forcing
				// that position to be a task. That's what makes `await`-in-loop
				// expressible: the loop body stays `nothing`-typed, while the
				// suspension still happens.
				new_constraints.push(eq_constraint(pattern_ty, payload_ty).at(try_range));
				new_constraints.push(eq_constraint(expr.ty.clone(), body_last_ty).at(try_range));

				// Soundness: a function that awaits must itself return a task (so its
				// callers see the right type and it lowers to an async closure). Tie
				// the enclosing async context's tail to a `task β`. For a tail-
				// position `try`-chain this is automatic (its tail is `task.return`);
				// the live case is a `try` whose continuation tail is *not* a task
				// (e.g. forgetting `task.return`) — that still errors here, exactly as
				// before. `scope` bodies carry their own task constraint, so the
				// walker passes `None` inside them.
				if let Some(tail) = enclosing_tail {
					let task_ty = Type::Enum("__prelude__.task".to_string(), vec![self.new_type_var()]);
					new_constraints.push(eq_constraint(tail.clone(), task_ty).at(try_range));
				}
				return true;
			}
		}

		let try_node = match std::mem::replace(&mut expr.kind, ExprKind::EmptyTuple) {
			ExprKind::Try(t) => t,
			_ => unreachable!("do_try_dispatch called on non-Try expr"),
		};

		let TryNode {
			range: try_range,
			pattern,
			value,
			rest,
			pattern_ty,
			..
		} = try_node;

		if rest.is_empty() {
			self.error(try_range, TryEmptyBody);
			expr.ty = Type::Unknown;
			return true;
		}

		// Recognized carriers: option (1 arg), result (2 args). task is
		// reserved for the post-async phase. Anything else is a user
		// error.
		let (carrier_module_name, payload_ty, err_ty): (&'static str, Type, Option<Type>) =
			match &resolved_value_ty {
				Type::Enum(name, args) if name == "__prelude__.option" && args.len() == 1 => {
					("option", args[0].clone(), None)
				}
				Type::Enum(name, args) if name == "__prelude__.result" && args.len() == 2 => {
					("result", args[0].clone(), Some(args[1].clone()))
				}
				_ => {
					self.error(
						try_range,
						TryUnsupportedCarrier {
							ty: resolved_value_ty.clone(),
						},
					);
					expr.ty = Type::Unknown;
					return true;
				}
			};

		// Pull a fun-param ident out of the LHS pattern. Identifier or
		// wildcard for now; richer patterns can desugar via an inner
		// `let` once that's needed.
		let param_ident = match &pattern.kind {
			PatternKind::Identifier(id) => id.clone(),
			PatternKind::Underscore => IdentifierNode {
				name: "_".to_string(),
				range: pattern.range,
			},
			_ => {
				self.error(try_range, TryUnsupportedPattern);
				expr.ty = Type::Unknown;
				return true;
			}
		};

		// Build constraints to link the existing tyvars with the
		// carrier-specific shape.
		//
		// 1. pattern_ty (the α the analyzer bound the LHS to during
		//    constrain) must equal the carrier's payload type.
		new_constraints.push(eq_constraint(pattern_ty.clone(), payload_ty.clone()).at(try_range));

		// 2. The continuation's tail expression must itself be
		//    carrier-wrapped (with the same err type, for result).
		let body_payload = self.new_type_var();
		let carrier_qualified = format!("__prelude__.{}", carrier_module_name);
		let expected_body_ty = match &err_ty {
			None => Type::Enum(carrier_qualified.clone(), vec![body_payload.clone()]),
			Some(e) => Type::Enum(
				carrier_qualified.clone(),
				vec![body_payload.clone(), e.clone()],
			),
		};
		let last_idx = rest.len() - 1;
		let body_last_ty = rest[last_idx].ty.clone();
		new_constraints
			.push(eq_constraint(body_last_ty.clone(), expected_body_ty.clone()).at(try_range));

		// 3. The whole `try` expression (now the synthesized Call) has
		//    that same carrier-wrapped type.
		new_constraints.push(eq_constraint(expr.ty.clone(), expected_body_ty.clone()).at(try_range));

		// Build the synthesized Fun (continuation closure).
		let body_end = rest.last().unwrap().range.end;
		let fun_range = Range::between(pattern.range.start, body_end);
		let fun_param_ty = pattern_ty.clone();
		let fun_node = FunNode {
			range: fun_range,
			params: vec![FunParamNode {
				ident: param_ident,
				ty: fun_param_ty.clone(),
			}],
			body: rest,
		};
		let fun_expr_ty = self.new_type_var();
		// Tie the synthesized Fun's type to Fun([pattern_ty], body_last_ty).
		new_constraints.push(
			eq_constraint(
				fun_expr_ty.clone(),
				Type::Fun(vec![fun_param_ty], Box::new(body_last_ty)),
			)
			.at(try_range),
		);
		let fun_expr = ExprNode {
			range: fun_range,
			kind: ExprKind::Fun(fun_node),
			ty: fun_expr_ty,
			trait_dispatch: None,
			dispatch_sink: None,
		};

		// Build the callee — a NamespaceAccess(["<carrier>", "then"]).
		let module_ident = IdentifierNode {
			name: carrier_module_name.to_string(),
			range: try_range,
		};
		let method_ident = IdentifierNode {
			name: "then".to_string(),
			range: try_range,
		};
		let callee = ExprNode {
			range: try_range,
			kind: ExprKind::NamespaceAccess(vec![module_ident, method_ident]),
			// `then`'s type doesn't strictly need to be set — codegen for
			// NamespaceAccess looks up the global by name. Annotate will
			// fill the placeholder if anything reads it.
			ty: self.new_type_var(),
			trait_dispatch: None,
			dispatch_sink: None,
		};

		expr.kind = ExprKind::Call(CallNode {
			range: try_range,
			callee: Box::new(callee),
			args: vec![*value, fun_expr],
			dict_args: Vec::new(),
		});

		true
	}

	// Rewrite one `??` BinaryOperation. The dual of `do_try_dispatch`:
	// `try` propagates failure via `<carrier>.then` (keeps the monad); `??`
	// recovers from failure via `<carrier>.or-else` (leaves the monad with a
	// bare value). `expr.kind` must be a `NullCoalescing` BinaryOperation on
	// entry. Returns `true` once handled (rewritten to a `Call`, or reported
	// and stubbed to `EmptyTuple`), `false` if the LHS type isn't pinned yet.
	fn do_coalesce_dispatch(
		&mut self,
		expr: &mut ExprNode,
		subst: &Substitution,
		new_constraints: &mut Vec<Constraint>,
	) -> bool {
		use AnalysisErrorKind::*;

		let coalesce_range = expr.range;

		// Peek the resolved LHS type; leave the node in place for a later
		// iteration if it's still a free tyvar.
		let left_ty = match &expr.kind {
			ExprKind::BinaryOperation { left, .. } => left.ty.clone(),
			_ => unreachable!("do_coalesce_dispatch called on non-BinaryOperation"),
		};
		let resolved_left = subst.apply_to_type(&left_ty);
		if matches!(resolved_left, Type::Var(_)) {
			return false;
		}

		// Take the node apart up front (like `do_try_dispatch`) so every
		// "handled" path — success or error — leaves `expr.kind` as something
		// other than a `??` BinaryOperation. Otherwise the dispatch fixpoint
		// would revisit this node forever.
		let (left, right) = match std::mem::replace(&mut expr.kind, ExprKind::EmptyTuple) {
			ExprKind::BinaryOperation { left, right, .. } => (left, right),
			_ => unreachable!("do_coalesce_dispatch called on non-BinaryOperation"),
		};

		// Recognized carriers mirror `try`: option (1 arg), result (2 args),
		// task (1 arg). For option/result, `??` *unwraps* to the bare payload;
		// for task it stays in the carrier (you can't synchronously unwrap a
		// task), so the "payload" the result type takes is the whole `task a`
		// and the fallback must itself be a `task a`.
		let (carrier_module_name, payload_ty): (&'static str, Type) = match &resolved_left {
			Type::Enum(name, args) if name == "__prelude__.option" && args.len() == 1 => {
				("option", args[0].clone())
			}
			Type::Enum(name, args) if name == "__prelude__.result" && args.len() == 2 => {
				("result", args[0].clone())
			}
			Type::Enum(name, args) if name == "__prelude__.task" && args.len() == 1 => {
				// Fully-qualified so codegen resolves it by name regardless of
				// imports: `core.task` isn't auto-imported, but `??` (like
				// `try`) must work on any task value -- including inside
				// `core.task` itself and in modules that only received a task
				// from elsewhere. The dot marks it compiler-inserted (user
				// namespace names are bare identifiers).
				("core.task", resolved_left.clone())
			}
			_ => {
				self.error(
					coalesce_range,
					CoalesceUnsupportedCarrier {
						ty: resolved_left.clone(),
					},
				);
				expr.ty = Type::Unknown;
				return true;
			}
		};

		// The unwrapped payload, the default's type (`right.ty`, already tied
		// to `expr.ty` during constrain), and the whole expression's type all
		// coincide. A mismatch (e.g. `option int ?? "s"`) surfaces here.
		new_constraints.push(eq_constraint(payload_ty, expr.ty.clone()).at(coalesce_range));

		// Wrap the default in a thunk so it's evaluated only on the failure
		// arm: `fun { default }` has type `fun nothing -> a`. This makes `??`
		// short-circuit, the dual of `then`'s lazy continuation.
		let right_range = right.range;
		let right_ty = right.ty.clone();
		let thunk_ty = self.new_type_var();
		new_constraints.push(
			eq_constraint(
				thunk_ty.clone(),
				Type::Fun(vec![Type::Nothing], Box::new(right_ty)),
			)
			.at(coalesce_range),
		);
		let thunk = ExprNode {
			range: right_range,
			kind: ExprKind::Fun(FunNode {
				range: right_range,
				params: vec![],
				body: vec![*right],
			}),
			ty: thunk_ty,
			trait_dispatch: None,
			dispatch_sink: None,
		};

		// Callee: NamespaceAccess(["<carrier>", "or-else"]). Resolved by name
		// in codegen, like the `try` rewrite's `then`.
		let module_ident = IdentifierNode {
			name: carrier_module_name.to_string(),
			range: coalesce_range,
		};
		let method_ident = IdentifierNode {
			name: "or-else".to_string(),
			range: coalesce_range,
		};
		let callee = ExprNode {
			range: coalesce_range,
			kind: ExprKind::NamespaceAccess(vec![module_ident, method_ident]),
			ty: self.new_type_var(),
			trait_dispatch: None,
			dispatch_sink: None,
		};

		expr.kind = ExprKind::Call(CallNode {
			range: coalesce_range,
			callee: Box::new(callee),
			args: vec![*left, thunk],
			dict_args: Vec::new(),
		});

		true
	}

	fn annotate(&mut self, module: &mut ModuleNode, subst: &Substitution) {
		for definition in &mut module.body {
			self.fill_in_placeholder(&mut definition.ty, subst);

			// The def itself is a statement with no type:
			definition.ty = Type::Nothing;

			match &mut definition.kind {
				DefinitionKind::Expr(expr) => {
					// But when defining exprs, we must annotate within the def value:
					self.annotate_expr(expr, subst);
				}
				DefinitionKind::Instance(instance_node) => {
					// Annotate each method body the same way regular defs are
					// annotated — the body's tyvars get substituted out.
					for method in &mut instance_node.methods {
						self.fill_in_placeholder(&mut method.ty, subst);
						method.ty = Type::Nothing;
						if let DefinitionKind::Expr(expr) = &mut method.kind {
							self.annotate_expr(expr, subst);
						}
					}
				}
				_ => { /* nothing to do for other def kinds */ }
			}
		}
	}

	fn annotate_expr(&mut self, expr: &mut ExprNode, subst: &Substitution) {
		self.fill_in_placeholder(&mut expr.ty, subst);
		let expr_range = expr.range;

		match &mut expr.kind {
			ExprKind::Let(LetNode { value, .. }) => {
				self.annotate_expr(value, subst);
			}

			ExprKind::Fun(FunNode { params, body, .. }) => {
				for param in params {
					self.fill_in_placeholder(&mut param.ty, subst);
				}

				for expr in body {
					self.annotate_expr(expr, subst);
				}
			}

			ExprKind::Call(CallNode {
				callee,
				args,
				dict_args,
				..
			}) => {
				self.annotate_expr(callee, subst);

				// Drain any cells the callee's dispatch sink collected during
				// Gen/Inst processing — these are the dicts this call needs
				// to prepend to its args at runtime.
				if let Some(sink) = callee.dispatch_sink.take() {
					dict_args.extend(sink.borrow().iter().cloned());
				}

				for arg in args {
					self.annotate_expr(arg, subst);
				}
			}

			ExprKind::Tuple(elements) => {
				for element in elements {
					self.annotate_expr(element, subst);
				}
			}

			ExprKind::List(items) => {
				for item in items {
					self.annotate_expr(item.expr_mut(), subst);
				}
			}

			ExprKind::Record(fields) => {
				for (_, field_value) in fields {
					self.annotate_expr(field_value, subst);
				}
			}

			ExprKind::RecordUpdate { base, fields } => {
				self.annotate_expr(base, subst);
				for (_, field_value) in fields {
					self.annotate_expr(field_value, subst);
				}
			}

			ExprKind::Interpolation(parts) => {
				for part in parts {
					self.annotate_expr(part, subst);
				}
			}

			ExprKind::ElementAccess { receiver, .. } => {
				self.annotate_expr(receiver, subst);
			}

			ExprKind::FieldAccess { receiver, .. } => {
				self.annotate_expr(receiver, subst);
			}

			ExprKind::UnaryOperation { right, .. } => {
				self.annotate_expr(right, subst);
			}

			ExprKind::BinaryOperation { left, right, .. } => {
				self.annotate_expr(left, subst);
				self.annotate_expr(right, subst);
			}

			ExprKind::When(WhenNode { subject, cases, .. }) => {
				self.annotate_expr(subject, subst);
				let subject_ty = subject.ty.clone();
				self.check_when_exhaustive(&subject_ty, cases, expr_range);
				for case in cases.iter_mut() {
					for body_expr in case.body.iter_mut() {
						self.annotate_expr(body_expr, subst);
					}
				}
			}

			ExprKind::If(IfNode {
				subject,
				body,
				else_body,
				..
			}) => {
				self.annotate_expr(subject, subst);
				for body_expr in body.iter_mut() {
					self.annotate_expr(body_expr, subst);
				}
				if let Some(else_body) = else_body {
					for else_expr in else_body.iter_mut() {
						self.annotate_expr(else_expr, subst);
					}
				}
			}

			ExprKind::While(WhileNode { subject, body, .. }) => {
				self.annotate_expr(subject, subst);
				for body_expr in body.iter_mut() {
					self.annotate_expr(body_expr, subst);
				}
			}

			ExprKind::Scope(ScopeNode { body, .. }) => {
				for body_expr in body.iter_mut() {
					self.annotate_expr(body_expr, subst);
				}
			}

			ExprKind::Grouping(inner) => {
				self.annotate_expr(inner, subst);
			}

			ExprKind::Defer(inner) => {
				self.annotate_expr(inner, subst);
			}

			ExprKind::Identifier(_) => {
				// nothing to annotate!
			}

			ExprKind::Literal(_) => {
				// nothing to annotate!
			}

			ExprKind::Regex(_) => {
				// nothing to annotate?
			}

			ExprKind::EmptyTuple => {
				// type is set during constrain; nothing to do here
			}

			ExprKind::NamespaceAccess(_) => {
				// path segments aren't typed (they're namespace names, not
				// values); the expr's own ty + dispatch metadata, set during
				// constrain, are all that needs annotating.
			}

			ExprKind::Try(TryNode {
				value,
				rest,
				pattern_ty,
				..
			}) => {
				// A `task`-carrier `try` survives to codegen (the CPS
				// lowering); fill in its inferred types. (option/result trys
				// were rewritten into `Call`s and never reach here; if one
				// did — dispatch failed — we still walk the sub-trees so
				// partial info is filled in.)
				self.fill_in_placeholder(pattern_ty, subst);
				self.annotate_expr(value, subst);
				for e in rest.iter_mut() {
					self.annotate_expr(e, subst);
				}
			}

			ExprKind::Builtin(_) => {
				// Type was set directly from the surrounding def's
				// annotation; nothing to fill in.
			}
		}
	}

	// Process each Inst against the scheme produced by Gen for `ty`. For
	// each: freshen the scheme's type into a new Eq constraint, freshen the
	// scheme's class constraints with new dispatch cells, and push the
	// fresh cells into the Inst's sink so the surrounding Call can read
	// them as `dict_args`. The new Eq + Class constraints are appended to
	// the constraint stream the unifier is processing.
	fn instantiate_constraints(
		&mut self,
		constraints: &[Constraint],
		ty: &Type,
		class_pool: &[ClassConstraint],
	) -> Vec<Constraint> {
		let mut new_constraints = Vec::new();

		let scheme = self.generalize_with_constraints(ty, class_pool);

		for constraint in constraints {
			if let Constraint::Inst(_, ty, sink, range) = constraint {
				let (instantiated_ty, fresh_class_constraints) =
					self.instantiate_scheme_with_constraints(&scheme);
				new_constraints.push(eq_constraint(ty.clone(), instantiated_ty).at(*range));

				// Each fresh class constraint comes with a fresh dispatch
				// cell — record them in the sink so the originating Call
				// can read them into its `dict_args`. We also stash a copy
				// of each fresh constraint on the analyzer so discharge
				// picks them up after unify finishes.
				//
				// Clear first: `unify` can run more than once (the `try`
				// dispatch fixpoint re-unifies with extra constraints), which
				// re-instantiates this same per-call-site sink. Without the
				// clear, each re-unify would *append* a duplicate set of dicts,
				// and the call would be handed too many `dict_args` at runtime
				// (an arity mismatch). The last pass's cells are the live ones —
				// they reference the final substitution's tyvars.
				{
					let mut sink_borrow = sink.borrow_mut();
					sink_borrow.clear();
					for c in &fresh_class_constraints {
						sink_borrow.push(c.dispatch_cell.clone());
					}
				}

				for c in fresh_class_constraints {
					self.fresh_class_constraints.push(c.clone());
					new_constraints.push(Constraint::Class(c));
				}
			} else {
				unreachable!("should only have inst constraints here");
			}
		}

		new_constraints
	}

	// Build the scheme that a given def's `ty` generalizes to, partitioning
	// class constraints from `class_pool` into "kept" (over the def's free
	// vars) vs. "passed through" (left in the surrounding context). The
	// `class_pool` here is whatever class constraints are still live at
	// the moment of generalization.
	fn generalize_with_constraints(&self, ty: &Type, class_pool: &[ClassConstraint]) -> Scheme {
		let mut free_vars: HashSet<usize> = HashSet::from_iter(ty.free_vars());
		// Free vars in the surrounding env aren't part of this scheme.
		for (_, binding) in self.value_scopes.last().unwrap() {
			for var in binding.ty_scheme.free_vars() {
				free_vars.remove(&var);
			}
		}
		// Class constraints whose ty mentions at least one of `free_vars`
		// belong to this scheme. Dedupe by `(trait, ty)` so multiple sites
		// over the same dispatch type collapse to one scheme slot —
		// matching `resolve_forwarded_dispatches`' slot allocation.
		let mut kept: Vec<ClassConstraint> = Vec::new();
		for c in class_pool {
			if !c.ty.free_vars().iter().any(|v| free_vars.contains(v)) {
				continue;
			}
			let already = kept
				.iter()
				.any(|k| k.name == c.name && type_keys_match(&k.ty, &c.ty));
			if !already {
				kept.push(c.clone());
			}
		}
		let mut free_row_vars: HashSet<usize> = ty.free_row_vars();
		for (_, binding) in self.value_scopes.last().unwrap() {
			for rv in binding.ty_scheme.free_row_vars() {
				free_row_vars.remove(&rv);
			}
		}
		Scheme::Forall(
			Vec::from_iter(free_vars),
			Vec::from_iter(free_row_vars),
			kept,
			ty.clone(),
		)
	}

	fn instantiate_scheme_with_constraints(
		&mut self,
		scheme: &Scheme,
	) -> (Type, Vec<ClassConstraint>) {
		match scheme {
			Scheme::Var(_) => unreachable!("shouldn't be instantiating a scheme var"),
			Scheme::Forall(vars, row_vars, class_constraints, ty) => {
				// generate a new fresh type var for each of the forall vars,
				// and a fresh row var for each quantified row var
				let mut subst = Substitution::empty();
				for var in vars {
					subst.solutions.insert(*var, self.new_type_var());
				}
				for rv in row_vars {
					let fresh = self.new_row_var();
					// A quantified row var is either a record tail or a tuple
					// tail — `free_row_vars` collects both into one set and we
					// can't tell them apart here. Redirect it in both maps; only
					// the one matching the type's actual shape is ever consulted.
					subst.row_solutions.insert(
						*rv,
						RowSolution {
							fields: vec![],
							tail: Some(fresh),
						},
					);
					subst.tuple_row_solutions.insert(
						*rv,
						TupleRowSolution {
							fields: vec![],
							tail: Some(fresh),
						},
					);
				}

				// and then apply that substitution in ty and the constraints
				let fresh_ty = subst.apply_to_type(ty);
				let fresh_constraints = class_constraints
					.iter()
					.map(|c| {
						let fresh_ty = subst.apply_to_type(&c.ty);
						ClassConstraint {
							name: c.name.clone(),
							ty: fresh_ty.clone(),
							reason: c.reason.clone(),
							// Each instantiation produces a *fresh* dispatch cell
							// that represents "the caller passing a dict" — NOT
							// the original method-extraction site. `method_idx`
							// is always None for these (the callee's own cells
							// do the method extraction once they receive the
							// dict).
							dispatch_cell: crate::ast::new_dispatch(c.name.clone(), None, fresh_ty),
						}
					})
					.collect();
				(fresh_ty, fresh_constraints)
			}
		}
	}

	fn new_type_scheme_var(&mut self) -> Scheme {
		let type_var = Scheme::Var(self.next_type_var_id);
		self.next_type_var_id += 1;
		type_var
	}

	fn new_type_var(&mut self) -> Type {
		let type_var = Type::Var(self.next_type_var_id);
		self.next_type_var_id += 1;
		type_var
	}

	// Row variables share the same counter as type variables; the two
	// spaces are kept distinct by where the id is *used* (type-var position
	// vs. record-tail position) and which substitution map binds it.
	fn new_row_var(&mut self) -> usize {
		let id = self.next_type_var_id;
		self.next_type_var_id += 1;
		id
	}

	// Lower a trait-method reference (either `trait.method` or bare
	// `method`) to a typed expression with a dispatch cell + Class
	// constraint. Caller has already picked the trait + method.
	fn emit_trait_method_dispatch(
		&mut self,
		trait_name: String,
		method_idx: usize,
		method_type: &Type,
		param_var: usize,
		expr: &mut ExprNode,
		constraints: &mut Vec<Constraint>,
	) {
		// Instantiate: replace the trait's param_var (and any other free
		// vars, defensively) with fresh vars at this use site.
		let mut mapping: HashMap<usize, Type> = HashMap::new();
		let dispatch_var = self.new_type_var();
		mapping.insert(param_var, dispatch_var.clone());
		let instantiated = self.instantiate_with(method_type, &mut mapping);

		expr.ty = instantiated;
		// Set up the shared dispatch cell + Class constraint. The cell is
		// the back-edge from constraint solving to the AST so codegen
		// knows which dict to load.
		let cell = crate::ast::new_dispatch(trait_name.clone(), Some(method_idx), dispatch_var.clone());
		expr.trait_dispatch = Some(cell.clone());
		constraints.push(Constraint::Class(ClassConstraint {
			name: trait_name,
			ty: dispatch_var,
			reason: ConstraintReason { range: expr.range },
			dispatch_cell: cell,
		}));
	}

	// Pick a single (trait, method_idx, method_type, param_var) from a list
	// of matches by precedence: local-module traits shadow everything; if
	// no local match, a single non-local match wins; otherwise return None
	// (caller reports ambiguity).
	fn disambiguate_bare_method_matches(
		&self,
		matches: &[(String, usize, Type, usize)],
	) -> Option<(String, usize, Type, usize)> {
		if matches.len() == 1 {
			return Some(matches[0].clone());
		}
		if let Some(module_name) = &self.module_name {
			let local: Vec<&(String, usize, Type, usize)> = matches
				.iter()
				.filter(|m| {
					self
						.traits
						.get(&m.0)
						.map(|d| &d.defining_module == module_name)
						.unwrap_or(false)
				})
				.collect();
			if local.len() == 1 {
				return Some(local[0].clone());
			}
			if local.len() > 1 {
				return None;
			}
		}
		None
	}

	// Pick a single (enum, variant) from a list of matches by precedence:
	// local-module enums shadow everything; if no local match, a single
	// non-local match wins; otherwise return None (caller reports ambiguity).
	fn disambiguate_variant_matches(&self, matches: &[(String, String)]) -> Option<(String, String)> {
		if matches.len() == 1 {
			return Some(matches[0].clone());
		}
		if let Some(module_name) = &self.module_name {
			let prefix = format!("{}.", module_name);
			let local: Vec<&(String, String)> = matches
				.iter()
				.filter(|(q, _)| q.starts_with(&prefix))
				.collect();
			if local.len() == 1 {
				return Some(local[0].clone());
			}
			if local.len() > 1 {
				return None;
			}
		}
		None
	}

	// Seed enums from a module's exports. Mints fresh local tyvars for
	// each enum's canonical-`Var(0..N-1)` params and substitutes through
	// the variant payload types. Always populates `enum_defs` and
	// `variant_constructors`; when `add_to_type_scope` is set, also adds
	// the bare enum name to `type_scope` so `option`/`ordering`/etc. can
	// be referenced unqualified (used for the implicit prelude import).
	fn seed_imported_enums(
		&mut self,
		qualified_module: &str,
		enums: &HashMap<String, EnumExport>,
		add_to_type_scope: bool,
	) {
		for (enum_name, enum_export) in enums {
			let qualified = format!("{}.{}", qualified_module, enum_name);
			let fresh_param_vars: Vec<usize> = (0..enum_export.param_count)
				.map(|_| {
					let id = self.next_type_var_id;
					self.next_type_var_id += 1;
					id
				})
				.collect();
			let rebind = Substitution {
				solutions: (0..enum_export.param_count)
					.map(|i| (i, Type::Var(fresh_param_vars[i])))
					.collect(),
				row_solutions: HashMap::new(),
				tuple_row_solutions: HashMap::new(),
			};
			let variants: Vec<(String, Vec<Type>)> = enum_export
				.variants
				.iter()
				.map(|(n, params)| {
					let rebound = params.iter().map(|p| rebind.apply_to_type(p)).collect();
					(n.clone(), rebound)
				})
				.collect();
			for (variant_name, _) in &variants {
				self
					.variant_constructors
					.entry(variant_name.clone())
					.or_default()
					.push((qualified.clone(), variant_name.clone()));
			}
			if add_to_type_scope {
				let template_args: Vec<Type> = fresh_param_vars.iter().map(|v| Type::Var(*v)).collect();
				self.add_type_binding(
					enum_name.clone(),
					Type::Enum(qualified.clone(), template_args),
					Range::collapsed(0, 0),
				);
			}
			self.enum_defs.insert(
				qualified,
				EnumDef {
					param_vars: fresh_param_vars,
					variants,
				},
			);
		}
	}

	// Pass 5 of analysis. After discharge, every concrete-typed dispatch
	// is already resolved. What's left are Forwarded dispatches — those
	// whose dispatch type is a tyvar bound by the enclosing top-level
	// def's generalized scheme. For each such def:
	//   - collect every dispatch cell in its body (trait_dispatch on each
	//     ExprNode + each Call's dict_args).
	//   - for each cell, apply `subst` to its `dispatch_var`; if the
	//     result is a Var, register that var as a dict-param slot.
	//   - allocate slot indices in first-seen order and write
	//     `Resolved::Forwarded(slot)` into each unresolved cell.
	//   - stash the slot count on `def.dict_param_count` so codegen knows
	//     how many hidden leading params to prepend.
	fn resolve_forwarded_dispatches(&mut self, module: &mut ModuleNode, subst: &Substitution) {
		for def in &mut module.body {
			match &mut def.kind {
				DefinitionKind::Expr(body_expr) => {
					// Collect every dispatch cell living in this def's body.
					let mut cells: Vec<DispatchCell> = Vec::new();
					collect_dispatch_cells(body_expr, &mut cells);

					// First-seen ordering of (trait, var_id) → slot index.
					// Lets callers and codegen agree on the dict-param layout
					// without any explicit signature carried around.
					let mut slot_order: Vec<(String, usize)> = Vec::new();

					for cell in &cells {
						let mut borrow = cell.borrow_mut();
						if borrow.resolved.is_some() {
							continue;
						}
						let resolved_ty = subst.apply_to_type(&borrow.dispatch_var);
						if let Type::Var(v) = resolved_ty {
							let slot = lookup_or_alloc_slot(&mut slot_order, &borrow.trait_name, v);
							borrow.resolved = Some(Resolved::Forwarded(slot));
						}
						// Cells whose dispatch type is concrete but unresolved have
						// already been errored on by `discharge`. Don't double-report.
					}

					// Merge in explicit `where (...)` constraints declared on
					// this def's signature. A `built-in` body carries no
					// dispatch cells, so this is the only source of dict params
					// for such defs; for an ordinary body it tops up the cells'
					// slots (de-duplicated by trait + var).
					if let Some(wcs) = self.def_where_clauses.get(&def.name.name) {
						for (trait_name, var_id) in wcs {
							if let Type::Var(v) = subst.apply_to_type(&Type::Var(*var_id)) {
								lookup_or_alloc_slot(&mut slot_order, trait_name, v);
							}
						}
					}

					def.dict_param_count = slot_order.len() as u16;
					if !slot_order.is_empty() {
						let exports: Vec<crate::module::ValueConstraintExport> = slot_order
							.iter()
							.map(|(trait_name, var)| crate::module::ValueConstraintExport {
								trait_name: trait_name.clone(),
								dispatch_var: Type::Var(*var),
							})
							.collect();
						self
							.def_value_constraints
							.insert(def.name.name.clone(), exports);
					}
				}

				DefinitionKind::Instance(_) => {
					// Slot ordering for parametric instances is fixed by the
					// declaration order of the `where` clauses. Look up the
					// registered InstanceDecl (its `where_clauses` carry the
					// canonical tyvars) and use them as the slot order.
					let slot_order = self.instance_slot_order_for(def);

					if let DefinitionKind::Instance(instance_node) = &mut def.kind {
						for method in &mut instance_node.methods {
							if let DefinitionKind::Expr(body) = &mut method.kind {
								let mut cells: Vec<DispatchCell> = Vec::new();
								collect_dispatch_cells(body, &mut cells);
								for cell in &cells {
									let mut borrow = cell.borrow_mut();
									if borrow.resolved.is_some() {
										continue;
									}
									let resolved_ty = subst.apply_to_type(&borrow.dispatch_var);
									if let Type::Var(v) = resolved_ty {
										if let Some(slot) = slot_order
											.iter()
											.position(|(t, sv)| t == &borrow.trait_name && *sv == v)
										{
											borrow.resolved = Some(Resolved::Forwarded(slot as u16));
										}
									}
								}
							}
						}
					}
				}

				_ => {}
			}
		}
	}

	// Look up the slot order for an instance def. Splits the borrow chain
	// from `resolve_forwarded_dispatches` so the loop can keep its
	// `&mut def.kind` while we read `self.instances`.
	fn instance_slot_order_for(&self, def: &DefinitionNode) -> Vec<(String, usize)> {
		if let DefinitionKind::Instance(instance_node) = &def.kind {
			let trait_name = &instance_node.trait_name.name;
			// Recompute head key from the instance's slot name suffix; the
			// slot was `<module>.<trait>@<head_key>` by construction.
			let head_key = instance_node
				.instance_slot_name
				.rsplit_once('@')
				.map(|(_, h)| h.to_string())
				.unwrap_or_default();
			if let Some(inst) = self.instances.get(&(trait_name.clone(), head_key.clone())) {
				return inst
					.where_clauses
					.iter()
					.map(|(t, v)| (t.clone(), *v))
					.collect();
			}
		}
		Vec::new()
	}

	// Shared helper: attach a numeric-trait dispatch cell to `expr` and
	// emit the corresponding Class constraint. Used by the operator-
	// desugaring branches (BinaryOperation arithmetic + UnaryOperation
	// negation). The dispatch type is whatever `alpha` resolves to after
	// unification — discharge picks the right int/float instance.
	fn emit_numeric_dispatch(
		&self,
		expr: &mut ExprNode,
		method_name: &str,
		alpha: &Type,
		constraints: &mut Vec<Constraint>,
	) {
		let trait_decl = self
			.traits
			.get("numeric")
			.expect("numeric trait must be registered in the prelude");
		let method_idx = trait_decl
			.method_order
			.iter()
			.position(|m| m == method_name)
			.expect("numeric method must be present");
		let cell = crate::ast::new_dispatch("numeric".into(), Some(method_idx), alpha.clone());
		expr.trait_dispatch = Some(cell.clone());
		constraints.push(Constraint::Class(ClassConstraint {
			name: "numeric".into(),
			ty: alpha.clone(),
			reason: ConstraintReason { range: expr.range },
			dispatch_cell: cell,
		}));
	}

	// Shared helper: attach an ord-trait `compare` dispatch cell to `expr`
	// and emit the corresponding Class constraint. Used by the ordering
	// operator desugaring (`<`, `>`, `<=`, `>=`).
	fn emit_ord_dispatch(
		&self,
		expr: &mut ExprNode,
		alpha: &Type,
		constraints: &mut Vec<Constraint>,
	) {
		let cell = crate::ast::new_dispatch("ord".into(), Some(0), alpha.clone());
		expr.trait_dispatch = Some(cell.clone());
		constraints.push(Constraint::Class(ClassConstraint {
			name: "ord".into(),
			ty: alpha.clone(),
			reason: ConstraintReason { range: expr.range },
			dispatch_cell: cell,
		}));
	}

	// Register the prelude `numeric` trait + `for numeric on int` and
	// `for numeric on float` instances. Method types reference the trait's
	// fresh `param_var` so each call-site instantiation can substitute the
	// dispatch type uniformly.
	fn register_prelude_numeric_trait(&mut self) {
		let param_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		let a = Type::Var(param_var);

		let binary = Type::Fun(vec![a.clone(), a.clone()], Box::new(a.clone()));
		let unary = Type::Fun(vec![a.clone()], Box::new(a.clone()));

		let method_order = vec![
			"add".to_string(),
			"sub".to_string(),
			"mul".to_string(),
			"div".to_string(),
			"negate".to_string(),
		];
		let mut method_types: HashMap<String, Type> = HashMap::new();
		method_types.insert("add".into(), binary.clone());
		method_types.insert("sub".into(), binary.clone());
		method_types.insert("mul".into(), binary.clone());
		method_types.insert("div".into(), binary.clone());
		method_types.insert("negate".into(), unary);

		self.traits.insert(
			"numeric".into(),
			TraitDecl {
				param_var,
				method_order,
				method_types,
				defaults: HashMap::new(),
				defining_module: "__prelude__".into(),
			},
		);

		self.instances.insert(
			("numeric".into(), "int".into()),
			InstanceDecl {
				trait_name: "numeric".into(),
				head_type: Type::Int,
				param_vars: vec![],
				where_clauses: vec![],
				instance_slot_name: "__prelude__.numeric@int".into(),
			},
		);
		self.instances.insert(
			("numeric".into(), "float".into()),
			InstanceDecl {
				trait_name: "numeric".into(),
				head_type: Type::Float,
				param_vars: vec![],
				where_clauses: vec![],
				instance_slot_name: "__prelude__.numeric@float".into(),
			},
		);
	}

	// Register the prelude `ord` trait + concrete instances on int, float,
	// and string. `compare`'s return type references the `ordering` prelude
	// enum we registered just above this call.
	fn register_prelude_ord_trait(&mut self) {
		let param_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		let a = Type::Var(param_var);

		let ordering_ty = Type::Enum("__prelude__.ordering".into(), vec![]);
		let compare_ty = Type::Fun(vec![a.clone(), a.clone()], Box::new(ordering_ty));

		let method_order = vec!["compare".to_string()];
		let mut method_types: HashMap<String, Type> = HashMap::new();
		method_types.insert("compare".into(), compare_ty);

		self.traits.insert(
			"ord".into(),
			TraitDecl {
				param_var,
				method_order,
				method_types,
				defaults: HashMap::new(),
				defining_module: "__prelude__".into(),
			},
		);

		for (head_key, head_type) in [
			("int", Type::Int),
			("float", Type::Float),
			("string", Type::String),
			("bytes", Type::Bytes),
		] {
			self.instances.insert(
				("ord".into(), head_key.into()),
				InstanceDecl {
					trait_name: "ord".into(),
					head_type,
					param_vars: vec![],
					where_clauses: vec![],
					instance_slot_name: format!("__prelude__.ord@{}", head_key),
				},
			);
		}
	}

	// Register the prelude `hash` trait + concrete instances on int,
	// float, string, bool. Output type is `int` — the analyzer doesn't
	// know about the hash algorithm; runtime semantics are in the
	// corresponding `*Hash` VM builtins.
	fn register_prelude_hash_trait(&mut self) {
		let param_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		let a = Type::Var(param_var);

		let hash_ty = Type::Fun(vec![a], Box::new(Type::Int));

		let method_order = vec!["hash".to_string()];
		let mut method_types: HashMap<String, Type> = HashMap::new();
		method_types.insert("hash".into(), hash_ty);

		self.traits.insert(
			"hash".into(),
			TraitDecl {
				param_var,
				method_order,
				method_types,
				defaults: HashMap::new(),
				defining_module: "__prelude__".into(),
			},
		);

		for (head_key, head_type) in [
			("int", Type::Int),
			("float", Type::Float),
			("string", Type::String),
			("bytes", Type::Bytes),
			("bool", Type::Bool),
		] {
			self.instances.insert(
				("hash".into(), head_key.into()),
				InstanceDecl {
					trait_name: "hash".into(),
					head_type,
					param_vars: vec![],
					where_clauses: vec![],
					instance_slot_name: format!("__prelude__.hash@{}", head_key),
				},
			);
		}
	}

	// Register the prelude `wire` trait: `encode fun a -> bytes` and
	// `decode fun bytes -> result a wire-error`. Unlike numeric/ord/hash,
	// `wire` registers NO instances — it's auto-derived structurally for any
	// data-shaped type (FULLSTACK.md, Layer 1), and `try_resolve_dispatch`
	// special-cases the trait to synthesize a schema rather than look up an
	// instance dictionary.
	fn register_prelude_wire_trait(&mut self) {
		let param_var = self.next_type_var_id;
		self.next_type_var_id += 1;
		let a = Type::Var(param_var);

		let encode = Type::Fun(vec![a.clone()], Box::new(Type::Bytes));
		let wire_error = Type::Enum("__prelude__.wire-error".into(), vec![]);
		let result_ty = Type::Enum("__prelude__.result".into(), vec![a.clone(), wire_error]);
		let decode = Type::Fun(vec![Type::Bytes], Box::new(result_ty));
		// `fingerprint a -> int`: the structural hash of `a`'s wire schema, for
		// version-skew detection (FULLSTACK.md). Takes a value only so it can
		// dispatch on `a`; the value itself is ignored.
		let fingerprint = Type::Fun(vec![a], Box::new(Type::Int));

		let method_order = vec![
			"encode".to_string(),
			"decode".to_string(),
			"fingerprint".to_string(),
		];
		let mut method_types: HashMap<String, Type> = HashMap::new();
		method_types.insert("encode".into(), encode);
		method_types.insert("decode".into(), decode);
		method_types.insert("fingerprint".into(), fingerprint);

		self.traits.insert(
			"wire".into(),
			TraitDecl {
				param_var,
				method_order,
				method_types,
				defaults: HashMap::new(),
				defining_module: "__prelude__".into(),
			},
		);
	}

	// Synthesize a `wire` schema shape for `ty`, or `None` if `ty` is not
	// auto-derivable (functions, refs, tasks, regex, dicts, opaque/empty
	// enums, open records, free type vars, or — for now — recursive enums).
	// This is both the derivability check (`is_some`) and the shape builder.
	// `visiting` holds the enum names currently being expanded, to break
	// recursive-type cycles (deferred; see M4).
	fn build_wire_shape(&self, ty: &Type, visiting: &mut Vec<String>) -> Option<WireShape> {
		match ty {
			Type::Int => Some(WireShape::Int),
			Type::Float => Some(WireShape::Float),
			Type::Bool => Some(WireShape::Bool),
			Type::String => Some(WireShape::Str),
			Type::Bytes => Some(WireShape::Bytes),
			Type::Duration => Some(WireShape::Duration),
			Type::Nothing => Some(WireShape::Nothing),
			Type::List(inner) => Some(WireShape::List(Box::new(
				self.build_wire_shape(inner, visiting)?,
			))),
			Type::Tuple(elems) => {
				let shapes = elems
					.iter()
					.map(|e| self.build_wire_shape(e, visiting))
					.collect::<Option<Vec<_>>>()?;
				Some(WireShape::Tuple(shapes))
			}
			// Only closed records (no row-variable tail) have a fully-known
			// field set; an open record can't be encoded positionally.
			Type::Record(fields, None) => {
				let mut sorted: Vec<&(String, Type)> = fields.iter().collect();
				sorted.sort_by(|a, b| a.0.cmp(&b.0));
				let out = sorted
					.into_iter()
					.map(|(name, fty)| Some((name.clone(), self.build_wire_shape(fty, visiting)?)))
					.collect::<Option<Vec<_>>>()?;
				Some(WireShape::Record(out))
			}
			Type::Enum(name, args) => {
				let def = self.enum_defs.get(name)?;
				// No variants => opaque or constructor-less (e.g. `task`):
				// can't synthesize encode/decode, so not derivable.
				if def.variants.is_empty() {
					return None;
				}
				// Recursive occurrence: cut the cycle with a by-name reference
				// (the codec resolves it against the enclosing inline def, which
				// is an ancestor in the shape). Keeps the schema finite.
				if visiting.iter().any(|n| n == name) {
					return Some(WireShape::EnumRef(name.clone()));
				}
				let mut mapping: HashMap<usize, Type> = HashMap::new();
				for (p, arg) in def.param_vars.iter().zip(args.iter()) {
					mapping.insert(*p, arg.clone());
				}
				let variants_src = def.variants.clone();
				visiting.push(name.clone());
				let mut variants = Vec::with_capacity(variants_src.len());
				for (vname, payloads) in &variants_src {
					let mut fields = Vec::with_capacity(payloads.len());
					for p in payloads {
						let concrete = subst_type(p, &mapping);
						fields.push(self.build_wire_shape(&concrete, visiting)?);
					}
					variants.push((vname.clone(), fields));
				}
				visiting.pop();
				Some(WireShape::Enum {
					qualified: name.clone(),
					variants,
				})
			}
			// `dict k v` wires as a sequence of (k, v) pairs. The key must be a
			// primitive so the codec can rehash it on decode (matching the
			// `hash` trait) without threading the key's hash instance; compound
			// keys are rejected.
			Type::Dict(k, v) if is_primitive_wire_key(k) => Some(WireShape::Dict(
				Box::new(self.build_wire_shape(k, visiting)?),
				Box::new(self.build_wire_shape(v, visiting)?),
			)),
			// Free type vars are handled by the forwarded-dict path (M3), not
			// here; everything else (Fun, Ref, Regex, dict-with-compound-key,
			// Instant, open record, PartialTuple/Record, Unknown) is
			// non-derivable.
			_ => None,
		}
	}

	// Explain why `ty` can't cross the wire, for the boundary diagnostic. Walks
	// the type for the first hard non-serializable component (function, ref,
	// regex, task, opaque enum, …) and describes it with attribution. Returns
	// `None` if the only obstacle is a free type variable — that's ambiguity
	// (annotate), not a serializability failure — so the caller falls back to
	// the generic ambiguity message.
	fn wire_underivable_detail(&self, ty: &Type, visiting: &mut Vec<String>) -> Option<String> {
		match ty {
			Type::Fun(..) => Some("functions aren't serializable".to_string()),
			Type::Ref(_) => Some("mutable refs aren't serializable".to_string()),
			Type::Regex => Some("regexes aren't serializable".to_string()),
			Type::Instant => Some("instants aren't serializable (send the value they wrap)".to_string()),
			// A dict with a compound key can't be rehashed by the codec; a
			// primitive-keyed dict is fine, so blame the value type instead.
			Type::Dict(k, v) => {
				if is_primitive_wire_key(k) {
					self.wire_underivable_detail(v, visiting)
				} else {
					Some("a dict needs an int/float/bool/string/bytes key to cross the wire".to_string())
				}
			}
			Type::List(inner) => self.wire_underivable_detail(inner, visiting),
			Type::Tuple(elems) => elems
				.iter()
				.find_map(|e| self.wire_underivable_detail(e, visiting)),
			Type::Record(fields, _) => fields.iter().find_map(|(name, t)| {
				self
					.wire_underivable_detail(t, visiting)
					.map(|d| format!("field `{}` can't ({})", name, d))
			}),
			Type::Enum(name, args) => {
				let Some(def) = self.enum_defs.get(name) else {
					return None;
				};
				let bare = name.rsplit('.').next().unwrap_or(name);
				if def.variants.is_empty() {
					return Some(if name.ends_with(".task") {
						"tasks aren't serializable".to_string()
					} else if name.contains("scope-handle") {
						"scope handles aren't serializable".to_string()
					} else {
						format!(
							"the opaque type `{}` hides its constructors — expose a non-opaque type, or send the value it wraps",
							bare
						)
					});
				}
				// Recursion is supported (cycle-cut with EnumRef), so it's not a
				// reason a type is non-derivable — stop descending this cycle and
				// keep looking elsewhere for the real blocker.
				if visiting.iter().any(|n| n == name) {
					return None;
				}
				let mut mapping: HashMap<usize, Type> = HashMap::new();
				for (p, arg) in def.param_vars.iter().zip(args.iter()) {
					mapping.insert(*p, arg.clone());
				}
				let variants = def.variants.clone();
				visiting.push(name.clone());
				let found = variants.iter().find_map(|(_, payloads)| {
					payloads.iter().find_map(|p| {
						let concrete = subst_type(p, &mapping);
						self.wire_underivable_detail(&concrete, visiting)
					})
				});
				visiting.pop();
				found
			}
			// Primitives are fine; a free `Var` is an ambiguity, not a hard
			// failure (handled by the caller).
			_ => None,
		}
	}

	// Build the enum type for a variant pattern: if the subject is already
	// `Type::Enum(name, args)` for the expected enum, reuse the subject's
	// args so pattern bindings see the concrete inner type. Otherwise mint
	// fresh vars per declared param so unification can pin them down.
	fn resolve_subject_enum_type(&mut self, subject_ty: &Type, expected_enum: &str) -> Type {
		if let Type::Enum(name, args) = subject_ty {
			if name == expected_enum {
				return Type::Enum(name.clone(), args.clone());
			}
		}
		let arity = self
			.enum_defs
			.get(expected_enum)
			.map(|d| d.param_vars.len())
			.unwrap_or(0);
		let args = (0..arity).map(|_| self.new_type_var()).collect();
		Type::Enum(expected_enum.to_string(), args)
	}

	// Resolve a variant lookup on an enum, minting fresh type vars for the
	// enum's params at this use site (so each `option.some` call sees its
	// own `a`). Returns the instantiated enum type (with fresh arg vars),
	// the variant's params (with the same substitution applied), and whether
	// the variant was found.
	fn instantiate_variant(
		&mut self,
		qualified_enum: &str,
		variant_name: &str,
		enum_def: &EnumDef,
	) -> (Type, Vec<Type>, Option<()>) {
		let mut mapping: HashMap<usize, Type> = HashMap::new();
		let fresh_args: Vec<Type> = enum_def
			.param_vars
			.iter()
			.map(|p| {
				let fresh = self.new_type_var();
				mapping.insert(*p, fresh.clone());
				fresh
			})
			.collect();
		let enum_ty = Type::Enum(qualified_enum.to_string(), fresh_args);

		match enum_def.variants.iter().find(|(n, _)| n == variant_name) {
			Some((_, params)) => {
				let instantiated = params
					.iter()
					.map(|p| self.instantiate_with(p, &mut mapping))
					.collect();
				(enum_ty, instantiated, Some(()))
			}
			None => (enum_ty, vec![], None),
		}
	}

	fn instantiate_with(&mut self, ty: &Type, mapping: &mut HashMap<usize, Type>) -> Type {
		match ty {
			Type::Var(n) => {
				if let Some(replacement) = mapping.get(n) {
					replacement.clone()
				} else {
					let fresh = self.new_type_var();
					mapping.insert(*n, fresh.clone());
					fresh
				}
			}
			Type::Fun(params, ret) => Type::Fun(
				params
					.iter()
					.map(|t| self.instantiate_with(t, mapping))
					.collect(),
				Box::new(self.instantiate_with(ret, mapping)),
			),
			Type::Tuple(elems) => Type::Tuple(
				elems
					.iter()
					.map(|t| self.instantiate_with(t, mapping))
					.collect(),
			),
			Type::Record(fields, tail) => Type::Record(
				fields
					.iter()
					.map(|(n, t)| (n.clone(), self.instantiate_with(t, mapping)))
					.collect(),
				*tail,
			),
			Type::Enum(name, args) => Type::Enum(
				name.clone(),
				args
					.iter()
					.map(|t| self.instantiate_with(t, mapping))
					.collect(),
			),
			Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Bytes
			| Type::Regex
			| Type::Instant
			| Type::Duration
			| Type::Unknown
			| Type::Nothing => ty.clone(),
			Type::PartialTuple(fields, tail) => Type::PartialTuple(
				fields
					.iter()
					.map(|(i, t)| (*i, self.instantiate_with(t, mapping)))
					.collect(),
				*tail,
			),
			Type::List(element_type) => {
				Type::List(Box::new(self.instantiate_with(element_type, mapping)))
			}
			Type::Dict(key_type, value_type) => Type::Dict(
				Box::new(self.instantiate_with(key_type, mapping)),
				Box::new(self.instantiate_with(value_type, mapping)),
			),
			Type::Ref(inner_type) => Type::Ref(Box::new(self.instantiate_with(inner_type, mapping))),
		}
	}
}

// Free function so both `constrain_pattern` and `constrain_let_pattern` can
// reuse it without fighting borrow-checker rules around mutably borrowing
// `self` and `fields` simultaneously. Reports the second and subsequent
// occurrences — the first one is the canonical site.
fn is_known_regex_character_class(name: &str) -> bool {
	matches!(name, "any" | "digit" | "letter" | "whitespace" | "word")
}

fn report_duplicate_record_pattern_fields(
	analyzer: &mut Analyzer,
	fields: &[(IdentifierNode, PatternNode)],
) {
	let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
	for (name, _) in fields {
		if !seen.insert(name.name.as_str()) {
			analyzer.error(
				name.range,
				AnalysisErrorKind::DuplicateRecordPatternField {
					field: name.name.clone(),
				},
			);
		}
	}
}
