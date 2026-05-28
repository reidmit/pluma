// Lowering: typed AST -> IR.
//
// This is where every backend-independent elaboration lives — the logic
// currently fused into `codegen/src/emit.rs`'s single AST->bytecode walk:
//   * identifier resolution (locals / captures / globals)
//   * closure conversion (explicit capture lists)
//   * dictionary elaboration (trait constraints -> dict params + GetDictMethod)
//   * pattern compilation (`when`/`if is` -> Switch + GetTag/GetPayload)
//   * `defer` edge insertion
//   * async marking (`Function::is_async` + `Await`)
//
// Phase 1.1 ports that elaboration here, function-by-function. Ported so far:
// the two standalone pre-passes (enum table + global reservation); literals,
// identifiers (local / capture / global), calls, `fun` (closure conversion),
// `let`; operators (direct opcodes + trait dispatch via method dictionaries);
// control flow (`if`/`when`/`while` via a pattern `Match`, with literal /
// variant / tuple / record / list patterns, nested and with `...` rests); data
// construction (variants + constructors, tuples, records, lists with spread,
// string interpolation, field access, regex literals); namespace access
// (`module.value`, `module.Enum.variant`) — which makes most stdlib calls work;
// and the full trait-dictionary machinery: instance defs (concrete +
// parametric), constrained defs (hidden dict params), every dispatch shape
// (`Global` / `Forwarded` / `InstanceChain`), constrained calls (dict args), and
// constrained-value references (dict-prepending wrappers).
// destructuring/`_` `let` (irrefutable patterns lowered as a single-arm match).
// Forms not yet handled (string-interpolation patterns, `defer`, async/`Await`,
// duration literals, ...) cause the *enclosing def* to be lowered as a poison
// thunk (returns `nothing`) rather than failing the whole program: a def whose
// executed paths only touch supported forms runs correctly, so coverage grows
// fixture-by-fixture. `lower` is not yet wired into `codegen` as the default.

use crate::types::*;
use compiler::ast::Resolved as DispatchTarget;
use compiler::ast::{
	DefinitionKind, ExprKind, ExprNode, FunNode, IfNode, LetNode, LiteralKind, ModuleNode, Operator,
	PatternKind, PatternNode, RegexAnchor, RegexKind, RegexNode, ScopeNode, TryNode, WhenNode,
	WhileNode,
};
use compiler::types::Type;
use compiler::{Compiler, Range};
use std::collections::HashMap;

/// A range used for stmts that have no source-level origin (entry function,
/// poison thunk, the dict-builder / ctor / constrained-ref wrappers).
const SYNTHETIC: Range = Range {
	start: compiler::Point { line: 0, col: 0 },
	end: compiler::Point { line: 0, col: 0 },
};

/// Lower a fully-analyzed program to IR.
///
/// Expects `compiler` to have completed `check()` (every module parsed and
/// analyzed, with inferred types attached to the AST). Returns `Err` only on a
/// structural failure (e.g. no `main`); individual defs that use not-yet-
/// supported constructs become poison thunks rather than failing the program.
pub fn lower(compiler: &Compiler) -> Result<IrProgram, String> {
	Lowerer::new(compiler).run()
}

// --------------------------------------------------------------------------
// The lowerer.
// --------------------------------------------------------------------------

struct Lowerer<'a> {
	compiler: &'a Compiler,
	enums: HashMap<String, Vec<(String, usize)>>,
	globals: GlobalTable,
	functions: Vec<Function>,
	// Active function nesting (innermost last). Pushed when descending into a
	// `fun`, popped when its body is done.
	scopes: Vec<FnScope>,
	// The module currently being lowered — used to resolve same-module globals
	// and bare variants.
	current_module: String,
	// Local-namespace-name -> qualified-module-name for the current module
	// (explicit `use`s plus auto-imports), for resolving `module.value` etc.
	imports: HashMap<String, String>,
	// A single shared thunk for every unsupported def, built lazily.
	poison: Option<FuncId>,
}

/// Per-function lowering state.
struct FnScope {
	name: String,
	module: String,
	params: Vec<VarId>,
	captures: Vec<CaptureInfo>,
	// Source name -> `VarId` for in-scope params and `let`s; searched
	// innermost-first (so a `let` shadows an earlier binding).
	locals: Vec<(String, VarId)>,
	next_var: u32,
	stmts: Vec<Stmt>,
	is_async: bool,
}

/// A free variable captured by a `fun`: the `VarId` it gets inside this
/// function, plus how the enclosing function supplies its value.
struct CaptureInfo {
	name: String,
	var: VarId,
	src: CaptureSrc,
}

enum CaptureSrc {
	/// A local (param or `let`) of the enclosing function.
	ParentLocal(VarId),
	/// A capture of the enclosing function (chained capture).
	ParentCapture(usize),
}

/// Result of resolving a name for use at an expression site.
enum Resolved {
	/// A local or capture — usable directly as an atom.
	Atom(Atom),
	Global(GlobalId),
	BareVariant {
		qualified: String,
		variant: String,
		arity: usize,
	},
}

/// Where a name resolves *within a particular scope* (the index-based form the
/// capture-chaining recursion threads through).
enum ScopeSlot {
	Local(VarId),
	Capture(usize),
	Global(GlobalId),
	BareVariant {
		qualified: String,
		variant: String,
		arity: usize,
	},
}

impl<'a> Lowerer<'a> {
	fn new(compiler: &'a Compiler) -> Self {
		let enums = collect_enums(compiler);
		let mut globals = GlobalTable::new();
		seed_prelude_globals(&mut globals);
		// Native modules currently contribute no globals — `vm::native_modules()`
		// is empty (every stdlib module is `.pa` source). When a Rust-defined
		// native module returns, its defs/constants are seeded here as `PreEval`.
		reserve_user_globals(&mut globals, compiler);
		Lowerer {
			compiler,
			enums,
			globals,
			functions: Vec::new(),
			scopes: Vec::new(),
			current_module: String::new(),
			imports: HashMap::new(),
			poison: None,
		}
	}

	fn run(mut self) -> Result<IrProgram, String> {
		// Copy the `&Compiler` out so the per-module borrow is independent of
		// `&mut self` in the loop body.
		let compiler = self.compiler;
		let modules: Vec<(&str, &ModuleNode)> = compiler
			.modules
			.iter()
			.filter_map(|(m, data)| data.ast.as_ref().map(|ast| (m.as_str(), ast)))
			.collect();
		for (module, ast) in modules {
			self.lower_module(module, ast);
		}

		let test_suites: Vec<(String, GlobalId)> = self
			.compiler
			.entry_modules
			.iter()
			.filter_map(|m| self.globals.lookup(m, "tests").map(|g| (m.clone(), g)))
			.collect();
		// A test-only program (suites but no `main`) gets a no-op entry; the
		// test runner drives each suite directly. `core.testing.new` is the
		// registrar it threads in — present whenever a suite is.
		let entry = self.build_entry(!test_suites.is_empty())?;
		let test_new = self.globals.lookup("core.testing", "new");

		// Annotate every function's bindings with a `Repr` (uniform-boxed except
		// arithmetic/comparison/`Not` results and primitive literals). Inert for the
		// bytecode VM; the WASM backend maps each repr to a native/GC-ref local.
		let mut functions = self.functions;
		for f in &mut functions {
			f.var_reprs = crate::repr::infer_reprs(f);
		}
		let enums = self.enums;
		let globals = self.globals.finish();
		Ok(IrProgram {
			functions,
			globals,
			enums,
			entry,
			test_suites,
			test_new,
		})
	}

	// ---- modules / defs ------------------------------------------------

	fn lower_module(&mut self, module: &str, ast: &ModuleNode) {
		self.current_module = module.to_string();
		self.imports = build_imports(ast);
		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(expr) => {
					self.lower_value_def(module, &def.name.name, def.dict_param_count, expr)
				}
				DefinitionKind::Alias(_) => {
					// Alias constructor (`fun x { x }`) — supported later.
					if let Some(g) = self.globals.lookup(module, &def.name.name) {
						self.poison_global(g);
					}
				}
				DefinitionKind::Instance(inst) => self.lower_instance(inst),
				DefinitionKind::Enum(_) | DefinitionKind::Trait(_) => {}
			}
		}
	}

	fn lower_value_def(&mut self, module: &str, name: &str, dict_param_count: u16, expr: &ExprNode) {
		let gid = match self.globals.lookup(module, name) {
			Some(g) => g,
			None => return,
		};
		// `built-in "tag"` RHS: a pre-evaluated builtin value, no thunk.
		if let ExprKind::Builtin(tag) = &expr.kind {
			self
				.globals
				.set_pre_evaluated(gid, PreEval::Builtin(tag.clone()));
			return;
		}
		// Trait-constrained def: hidden leading dict params. Lower to an inner
		// K+N-arity function wrapped in a thunk returning its closure.
		if dict_param_count > 0 {
			match self.lower_constrained_def(name, dict_param_count, expr) {
				Ok(fid) => self.globals.set_thunk(gid, fid),
				Err(_) => self.poison_global(gid),
			}
			return;
		}
		match self.lower_thunk(name, expr) {
			Ok(fid) => self.globals.set_thunk(gid, fid),
			Err(_) => self.poison_global(gid),
		}
	}

	/// A def's value thunk: a zero-arg function that evaluates `expr` and
	/// returns it. `expr` is in tail position (its value is the thunk's return).
	fn lower_thunk(&mut self, name: &str, expr: &ExprNode) -> Result<FuncId, String> {
		let fn_name = format!("{}.{}@thunk", self.current_module, name);
		self.push_scope(fn_name, &[]);
		if let Err(e) = self.lower_tail(expr) {
			self.scopes.pop();
			return Err(e);
		}
		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	// ---- trait instances / constrained defs ----------------------------

	/// Lower a trait `instance` def to its method-dictionary global. Mirrors
	/// `codegen::emit::compile_instance`; a build failure poisons the slot.
	fn lower_instance(&mut self, instance: &compiler::ast::InstanceNode) {
		let gid = match instance.instance_slot_name.rsplit_once('.') {
			Some((m, n)) => self.globals.lookup(m, n),
			None => None,
		};
		let Some(gid) = gid else { return };
		match self.lower_instance_thunk(instance) {
			Ok(fid) => self.globals.set_thunk(gid, fid),
			Err(_) => self.poison_global(gid),
		}
	}

	fn lower_instance_thunk(
		&mut self,
		instance: &compiler::ast::InstanceNode,
	) -> Result<FuncId, String> {
		// Index method bodies by name so they can be emitted in canonical order.
		let mut by_name: HashMap<&str, &ExprNode> = HashMap::new();
		for m in &instance.methods {
			if let DefinitionKind::Expr(e) = &m.kind {
				by_name.insert(m.name.name.as_str(), e);
			}
		}

		if instance.where_clause.is_empty() {
			// Concrete: a zero-arg thunk that builds the dict directly. Each
			// method `fun` lowers to a closure with no captures.
			let name = format!("{}@dict-builder", instance.instance_slot_name);
			self.push_scope(name, &[]);
			match self.build_dict_body(instance, &by_name) {
				Ok(()) => {
					let scope = self.scopes.pop().unwrap();
					Ok(self.add_function(finish_scope(scope)))
				}
				Err(e) => {
					self.scopes.pop();
					Err(e)
				}
			}
		} else {
			// Parametric: a ctor of arity K (the `where`-clause dicts), with the
			// dicts bound as synthetic locals so each method closure captures the
			// ones it forwards to. The global is a thunk returning the ctor's
			// closure; `InstanceChain` sites call it with the inner dicts.
			let k = instance.where_clause.len();
			let dict_names: Vec<String> = (0..k).map(|n| synthetic_dict_name(n as u16)).collect();
			let dict_refs: Vec<&str> = dict_names.iter().map(String::as_str).collect();
			let ctor_name = format!("{}@ctor", instance.instance_slot_name);
			self.push_scope(ctor_name, &dict_refs);
			let ctor_fid = match self.build_dict_body(instance, &by_name) {
				Ok(()) => {
					let scope = self.scopes.pop().unwrap();
					self.add_function(finish_scope(scope))
				}
				Err(e) => {
					self.scopes.pop();
					return Err(e);
				}
			};
			let builder_name = format!("{}@builder", instance.instance_slot_name);
			self.push_scope(builder_name, &[]);
			let c = self.emit_let(Rvalue::MakeClosure(ctor_fid, Vec::new()), SYNTHETIC);
			self.push_synthetic(StmtKind::Return(c));
			let scope = self.scopes.pop().unwrap();
			Ok(self.add_function(finish_scope(scope)))
		}
	}

	/// Lower each method (in canonical/trait order) into the current scope and
	/// `Return` a `MakeDict` of the resulting method closures. For parametric
	/// instances the methods capture the enclosing ctor's dict params via
	/// `Forwarded` dispatch automatically.
	fn build_dict_body(
		&mut self,
		instance: &compiler::ast::InstanceNode,
		by_name: &HashMap<&str, &ExprNode>,
	) -> Result<(), String> {
		let mut methods = Vec::with_capacity(instance.canonical_method_order.len());
		for method_name in &instance.canonical_method_order {
			let expr: &ExprNode = by_name.get(method_name.as_str()).copied().ok_or_else(|| {
				format!(
					"instance `{}` is missing method `{}`",
					instance.instance_slot_name, method_name
				)
			})?;
			methods.push(self.lower_expr(expr)?);
		}
		let dict = self.emit_let(Rvalue::MakeDict(methods), SYNTHETIC);
		self.push_synthetic(StmtKind::Return(dict));
		Ok(())
	}

	/// Lower a trait-constrained def (one with `dict_param_count` hidden leading
	/// dict params). The body is always a `fun`; lower it as a single inner
	/// function of arity K+N (dicts at slots 0..K-1 under synthetic names so
	/// `Forwarded` dispatch resolves them), wrapped in a thunk that returns its
	/// closure. Mirrors `codegen::emit::compile_constrained_thunk`.
	fn lower_constrained_def(
		&mut self,
		name: &str,
		dict_param_count: u16,
		expr: &ExprNode,
	) -> Result<FuncId, String> {
		let fun = match &expr.kind {
			ExprKind::Fun(f) => f,
			_ => {
				return Err(format!(
					"constrained def `{}` must have a function body",
					name
				))
			}
		};
		let k = dict_param_count as usize;
		let dict_names: Vec<String> = (0..k).map(|n| synthetic_dict_name(n as u16)).collect();
		let mut param_names: Vec<&str> = dict_names.iter().map(String::as_str).collect();
		param_names.extend(fun.params.iter().map(|p| p.ident.name.as_str()));

		let inner_name = format!("{}.{}", self.current_module, name);
		self.push_scope(inner_name, &param_names);
		let body_range = fun.body.last().map(|e| e.range).unwrap_or(fun.range);
		let inner_fid = match self.lower_body_tail(&fun.body, body_range) {
			Ok(()) => {
				let scope = self.scopes.pop().unwrap();
				self.add_function(finish_scope(scope))
			}
			Err(e) => {
				self.scopes.pop();
				return Err(e);
			}
		};

		// Thunk: return a closure of the inner function (no captures).
		let thunk_name = format!("{}.{}@thunk", self.current_module, name);
		self.push_scope(thunk_name, &[]);
		let c = self.emit_let(Rvalue::MakeClosure(inner_fid, Vec::new()), SYNTHETIC);
		self.push_synthetic(StmtKind::Return(c));
		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	/// A constrained function referenced as a first-class value: wrap it in a
	/// closure of its user-visible arity N that captures the K resolved dicts
	/// and forwards to the underlying global with the dicts prepended. Mirrors
	/// `codegen::emit::emit_constrained_value_ref`.
	fn lower_constrained_value_ref(
		&mut self,
		expr: &ExprNode,
		cells: &[compiler::ast::DispatchCell],
	) -> Result<Atom, String> {
		let k = cells.len() as u32;
		let n = match &expr.ty {
			Type::Fun(params, _) => params.len() as u32,
			_ => return Err("constrained value reference has a non-function type".to_string()),
		};
		let global = self.resolve_constrained_ref_global(expr)?;

		// Wrapper: N params, K dict captures. Var numbering — params 0..N-1,
		// captures N..N+K-1, then two `let`s for the load + the forwarded call.
		let params: Vec<VarId> = (0..n).map(VarId).collect();
		let captures: Vec<VarId> = (n..n + k).map(VarId).collect();
		let g_var = VarId(n + k);
		let r_var = VarId(n + k + 1);
		let mut call_args: Vec<Atom> = captures.iter().map(|v| Atom::Var(*v)).collect();
		call_args.extend(params.iter().map(|v| Atom::Var(*v)));
		let wrapper = Function {
			name: format!("{}.partial@{}", self.current_module, global.0),
			module: self.current_module.clone(),
			params,
			captures,
			is_async: false,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(g_var, Rvalue::GlobalRef(global))),
				Stmt::synthetic(StmtKind::Let(
					r_var,
					Rvalue::CallClosure(Atom::Var(g_var), call_args),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(r_var))),
			]),
			var_reprs: Vec::new(),
		};
		let wrapper_fid = self.add_function(wrapper);

		// Outer site: load each resolved dict (so `Forwarded` dicts capture the
		// enclosing def's dict params), then build the wrapper closure.
		let mut dict_atoms = Vec::with_capacity(cells.len());
		for cell in cells {
			dict_atoms.push(self.lower_dispatch(cell, expr.range)?);
		}
		Ok(self.emit_let(Rvalue::MakeClosure(wrapper_fid, dict_atoms), expr.range))
	}

	/// Resolve the underlying global of a constrained value reference — a bare
	/// identifier or an imported `module.value`.
	fn resolve_constrained_ref_global(&mut self, expr: &ExprNode) -> Result<GlobalId, String> {
		match &expr.kind {
			ExprKind::Identifier(id) => match self.resolve(&id.name)? {
				Resolved::Global(g) => Ok(g),
				_ => Err(format!(
					"constrained value `{}` did not resolve to a global",
					id.name
				)),
			},
			ExprKind::NamespaceAccess(path) => match path.as_slice() {
				[head, tail] => {
					let qualified_module = self
						.imports
						.get(&head.name)
						.cloned()
						.ok_or_else(|| format!("`{}` is not an imported module", head.name))?;
					self
						.globals
						.lookup(&qualified_module, &tail.name)
						.ok_or_else(|| format!("`{}.{}` is not a global", head.name, tail.name))
				}
				_ => Err("constrained value reference namespace path".to_string()),
			},
			_ => Err("constrained value reference is neither identifier nor namespace".to_string()),
		}
	}

	// ---- expressions ---------------------------------------------------

	fn lower_expr(&mut self, expr: &ExprNode) -> Result<Atom, String> {
		let range = expr.range;
		match &expr.kind {
			ExprKind::Literal(lit) => Ok(Atom::Const(literal_to_const(&lit.kind)?)),
			ExprKind::Grouping(inner) => self.lower_expr(inner),
			ExprKind::EmptyTuple => Ok(Atom::Const(Const::Unit)),
			ExprKind::Identifier(id) => {
				// A bare trait-method reference (`hash`) carries a dispatch cell.
				if let Some(cell) = &expr.trait_dispatch {
					return self.lower_dispatch(cell, range);
				}
				if let Some(cells) = undrained_dispatch_cells(expr) {
					return self.lower_constrained_value_ref(expr, &cells);
				}
				self.lower_identifier(&id.name, range)
			}
			ExprKind::Call(call) => self.lower_call(call, range),
			ExprKind::Tuple(elems) => {
				let mut atoms = Vec::with_capacity(elems.len());
				for e in elems {
					atoms.push(self.lower_expr(e)?);
				}
				Ok(self.emit_let(Rvalue::MakeTuple(atoms), range))
			}
			ExprKind::List(items) => {
				let mut ir_items = Vec::with_capacity(items.len());
				for item in items {
					let atom = self.lower_expr(item.expr())?;
					ir_items.push(if item.is_spread() {
						ListItem::Spread(atom)
					} else {
						ListItem::Elem(atom)
					});
				}
				Ok(self.emit_let(Rvalue::MakeList(ir_items), range))
			}
			ExprKind::Record(fields) => {
				let mut ir_fields = Vec::with_capacity(fields.len());
				for (name, value) in fields {
					let atom = self.lower_expr(value)?;
					ir_fields.push((name.name.clone(), atom));
				}
				Ok(self.emit_let(Rvalue::MakeRecord(ir_fields), range))
			}
			ExprKind::Interpolation(parts) => {
				let mut atoms = Vec::with_capacity(parts.len());
				for p in parts {
					atoms.push(self.lower_expr(p)?);
				}
				Ok(self.emit_let(Rvalue::Interpolate(atoms), range))
			}
			ExprKind::FieldAccess { receiver, field } => {
				// Record field access (namespace/variant shapes are
				// `NamespaceAccess` by this point). If `receiver` is actually a
				// namespace it won't lower as a value, poisoning the def.
				let recv = self.lower_expr(receiver)?;
				Ok(self.emit_let(Rvalue::GetField(recv, field.name.clone()), range))
			}
			ExprKind::NamespaceAccess(path) => {
				if let Some(cell) = &expr.trait_dispatch {
					return self.lower_dispatch(cell, range);
				}
				if let Some(cells) = undrained_dispatch_cells(expr) {
					return self.lower_constrained_value_ref(expr, &cells);
				}
				self.lower_namespace(path, range)
			}
			ExprKind::Fun(fun) => self.lower_fun(fun, range),
			ExprKind::BinaryOperation { op, left, right } => {
				self.lower_binary(expr.trait_dispatch.as_ref(), &op.kind, left, right, range)
			}
			ExprKind::UnaryOperation { op, right } => {
				self.lower_unary(expr.trait_dispatch.as_ref(), op, right, range)
			}
			ExprKind::If(n) => self.lower_if(n, range),
			ExprKind::When(n) => self.lower_when(n, range),
			ExprKind::While(n) => self.lower_while(n, range),
			ExprKind::Regex(node) => Ok(self.emit_let(Rvalue::Regex(regex_pattern(node)), range)),
			ExprKind::Defer(inner) => self.lower_defer(inner, range),
			ExprKind::Try(node) => self.lower_try(node, range),
			ExprKind::Scope(node) => self.lower_scope(node, range),
			other => Err(format!("unsupported expr: {}", expr_kind_name(other))),
		}
	}

	fn lower_identifier(&mut self, name: &str, range: Range) -> Result<Atom, String> {
		match self.resolve(name)? {
			Resolved::Atom(a) => Ok(a),
			Resolved::Global(g) => Ok(self.emit_let(Rvalue::GlobalRef(g), range)),
			Resolved::BareVariant {
				qualified,
				variant,
				arity,
			} => self.make_variant_ref(&qualified, &variant, arity, range),
		}
	}

	/// A bare reference to a variant: a finished value for a nullary variant,
	/// or a constructor value for one with payload.
	fn make_variant_ref(
		&mut self,
		enum_name: &str,
		variant: &str,
		arity: usize,
		range: Range,
	) -> Result<Atom, String> {
		if arity == 0 {
			self.make_variant(enum_name, variant, Vec::new(), range)
		} else {
			let tag = self
				.variant_tag(enum_name, variant)
				.ok_or_else(|| format!("unknown variant `{}` of `{}`", variant, enum_name))?;
			Ok(self.emit_let(
				Rvalue::MakeVariantCtor {
					enum_name: enum_name.to_string(),
					tag,
				},
				range,
			))
		}
	}

	/// Construct an enum variant (looking up its tag in the enum table).
	fn make_variant(
		&mut self,
		enum_name: &str,
		variant: &str,
		payload: Vec<Atom>,
		range: Range,
	) -> Result<Atom, String> {
		let tag = self
			.variant_tag(enum_name, variant)
			.ok_or_else(|| format!("unknown variant `{}` of `{}`", variant, enum_name))?;
		Ok(self.emit_let(
			Rvalue::MakeVariant {
				enum_name: enum_name.to_string(),
				tag,
				payload,
			},
			range,
		))
	}

	/// Lower a `NamespaceAccess` path: `module.value` (-> a global),
	/// `module.Enum.variant` / `Enum.variant` (-> variant construction), or a
	/// compiler-inserted fully-qualified `mod.name` reference.
	fn lower_namespace(
		&mut self,
		path: &[compiler::ast::IdentifierNode],
		range: Range,
	) -> Result<Atom, String> {
		match path {
			[module, enum_name, variant] => {
				let qualified_module = self
					.imports
					.get(&module.name)
					.ok_or_else(|| format!("`{}` is not an imported module", module.name))?
					.clone();
				let qualified_enum = format!("{}.{}", qualified_module, enum_name.name);
				let arity = self.variant_arity(&qualified_enum, &variant.name)?;
				self.make_variant_ref(&qualified_enum, &variant.name, arity, range)
			}
			[head, tail] => {
				// A dotted head is an already-fully-qualified reference (e.g.
				// `core.task.or-else`), resolved directly against globals.
				if head.name.contains('.') {
					if let Some(g) = self.globals.lookup(&head.name, &tail.name) {
						return Ok(self.emit_let(Rvalue::GlobalRef(g), range));
					}
					return Err(format!("`{}.{}` not found", head.name, tail.name));
				}
				// `module.value`.
				if let Some(qualified_module) = self.imports.get(&head.name).cloned() {
					if let Some(g) = self.globals.lookup(&qualified_module, &tail.name) {
						return Ok(self.emit_let(Rvalue::GlobalRef(g), range));
					}
				}
				// `Enum.variant` for a local-module enum.
				let qualified_enum = format!("{}.{}", self.current_module, head.name);
				if self.enums.contains_key(&qualified_enum) {
					let arity = self.variant_arity(&qualified_enum, &tail.name)?;
					return self.make_variant_ref(&qualified_enum, &tail.name, arity, range);
				}
				Err(format!(
					"`{}.{}` is neither an imported value nor a local variant",
					head.name, tail.name
				))
			}
			_ => Err(format!("namespace path with {} segments", path.len())),
		}
	}

	fn variant_arity(&self, enum_name: &str, variant: &str) -> Result<usize, String> {
		self
			.enums
			.get(enum_name)
			.and_then(|vs| vs.iter().find(|(n, _)| n == variant))
			.map(|(_, a)| *a)
			.ok_or_else(|| format!("variant `{}` not in `{}`", variant, enum_name))
	}

	fn variant_tag(&self, enum_name: &str, variant: &str) -> Option<u32> {
		self
			.enums
			.get(enum_name)?
			.iter()
			.position(|(n, _)| n == variant)
			.map(|i| i as u32)
	}

	fn lower_call(&mut self, call: &compiler::ast::CallNode, range: Range) -> Result<Atom, String> {
		let callee = self.lower_expr(&call.callee)?;
		let mut args = Vec::with_capacity(call.dict_args.len() + call.args.len());
		// Hidden dict args precede the user args — the constrained callee
		// expects them at slots 0..K-1. A call-forwarding cell has
		// `method_idx == None`, so `lower_dispatch` pushes the whole dict.
		for cell in &call.dict_args {
			args.push(self.lower_dispatch(cell, range)?);
		}
		for a in &call.args {
			args.push(self.lower_expr(a)?);
		}
		Ok(self.emit_let(Rvalue::CallClosure(callee, args), range))
	}

	fn lower_binary(
		&mut self,
		cell: Option<&compiler::ast::DispatchCell>,
		op: &Operator,
		left: &ExprNode,
		right: &ExprNode,
		range: Range,
	) -> Result<Atom, String> {
		// Trait-dispatched operator: the method comes from a method dictionary
		// (e.g. `+` is `numeric.add`). Arithmetic is `method(left, right)`;
		// ordering (`< <= > >=`) needs an extra `compare(...) {==,!=} <variant>`
		// tail (deferred until variant construction lands).
		if let Some(cell) = cell {
			match op {
				Operator::Addition
				| Operator::SubtractionOrNegation
				| Operator::Multiplication
				| Operator::Division
				| Operator::Remainder => {
					// Devirtualize: when both operands are a concrete numeric type
					// (`int`/`float`), the `numeric` instance is statically known, so
					// emit the direct VM opcode (`AddInt`, `DivFloat`, …) instead of a
					// boxed dispatch through the method dictionary. Each opcode is
					// byte-identical to the dict's builtin method (`int-add` == `AddInt`,
					// …; `DivInt` is aligned to `int-div` in the VM), so this is
					// behavior-preserving — it just drops the dict load and the closure
					// call. Polymorphic operands (a `numeric a` type variable inside a
					// constrained def) fall through to dispatch, since their instance
					// arrives in the hidden dict parameter at runtime.
					if let Some(binop) = concrete_numeric_binop(op, &left.ty, &right.ty) {
						let l = self.lower_expr(left)?;
						let r = self.lower_expr(right)?;
						return Ok(self.emit_let(Rvalue::Bin(binop, l, r), range));
					}
					let method = self.lower_dispatch(cell, range)?;
					let l = self.lower_expr(left)?;
					let r = self.lower_expr(right)?;
					return Ok(self.emit_let(Rvalue::CallClosure(method, vec![l, r]), range));
				}
				Operator::LessThan
				| Operator::LessThanEquals
				| Operator::GreaterThan
				| Operator::GreaterThanEquals => {
					// Devirtualize concrete comparisons to the direct ordering opcodes
					// (`LtI64`/`LeF64`/…), dropping a dict load, a closure call, and a
					// variant construction. For concrete floats these are IEEE-754
					// comparisons (NaN -> false for all four relations) — the language's
					// defined semantics for concrete float relational operators. Generic
					// (polymorphic) comparisons keep the `ord.compare` total order below.
					// See `concrete_ord_binop`.
					if let Some(binop) = concrete_ord_binop(op, &left.ty, &right.ty) {
						let l = self.lower_expr(left)?;
						let r = self.lower_expr(right)?;
						return Ok(self.emit_let(Rvalue::Bin(binop, l, r), range));
					}
					// `ord.compare(left, right)` then test the resulting ordering
					// variant: `< == lt`, `> == gt`, `<= != gt`, `>= != lt`.
					let method = self.lower_dispatch(cell, range)?;
					let l = self.lower_expr(left)?;
					let r = self.lower_expr(right)?;
					let cmp = self.emit_let(Rvalue::CallClosure(method, vec![l, r]), range);
					let (variant, use_ne) = match op {
						Operator::LessThan => ("lt", false),
						Operator::GreaterThan => ("gt", false),
						Operator::LessThanEquals => ("gt", true),
						Operator::GreaterThanEquals => ("lt", true),
						_ => unreachable!(),
					};
					let v = self.make_variant("__prelude__.ordering", variant, Vec::new(), range)?;
					let binop = if use_ne { BinOp::Ne } else { BinOp::Eq };
					return Ok(self.emit_let(Rvalue::Bin(binop, cmp, v), range));
				}
				_ => return Err("unsupported dispatched operator".to_string()),
			}
		}
		// `x | f a b` pipes `x` in as `f`'s first argument.
		if let Operator::Chain = op {
			return self.lower_chain(left, right, range);
		}
		// Concrete, non-dispatched operator: a direct VM opcode picked by
		// operand type. Evaluate left then right (matching `emit.rs`).
		let is_float = matches!(left.ty, Type::Float) || matches!(right.ty, Type::Float);
		let binop = binop_for(op, is_float).ok_or("unsupported binary operator")?;
		let l = self.lower_expr(left)?;
		let r = self.lower_expr(right)?;
		Ok(self.emit_let(Rvalue::Bin(binop, l, r), range))
	}

	fn lower_unary(
		&mut self,
		cell: Option<&compiler::ast::DispatchCell>,
		op: &Operator,
		right: &ExprNode,
		range: Range,
	) -> Result<Atom, String> {
		// Numeric `-` (negate) is trait-dispatched (`numeric.negate`); the only
		// direct unary op is logical `!`.
		if let Some(cell) = cell {
			let method = self.lower_dispatch(cell, range)?;
			let r = self.lower_expr(right)?;
			return Ok(self.emit_let(Rvalue::CallClosure(method, vec![r]), range));
		}
		match op {
			Operator::LogicalNot => {
				let r = self.lower_expr(right)?;
				Ok(self.emit_let(Rvalue::Not(r), range))
			}
			_ => Err("unsupported unary operator".to_string()),
		}
	}

	/// Lower a resolved trait-method dispatch cell to its value: the dict if the
	/// cell is call-forwarding (`method_idx == None`), or a specific method
	/// extracted from it. Mirrors `codegen::emit::emit_dispatch_load`.
	fn lower_dispatch(
		&mut self,
		cell: &compiler::ast::DispatchCell,
		range: Range,
	) -> Result<Atom, String> {
		let borrow = cell.borrow();
		let method_idx = borrow.method_idx;
		let resolved = borrow.resolved.clone().ok_or("unresolved dispatch cell")?;
		drop(borrow);
		let dict = self.lower_dict_atom(&resolved, range)?;
		match method_idx {
			Some(idx) => Ok(self.emit_let(Rvalue::GetDictMethod(dict, idx as u32), range)),
			// A call-forwarding site pushes the whole dict; operators always
			// name a method, so this branch is unused for them.
			None => Ok(dict),
		}
	}

	/// Load a dispatch dictionary value (no method extraction). The three
	/// `Resolved` shapes mirror `codegen::emit::emit_resolved_load`:
	///   * `Global` — load the named prelude/instance dict global.
	///   * `Forwarded` — the synthetic `__dict_<slot>__` local of the enclosing
	///     constrained def / instance ctor (captured through closures by name).
	///   * `InstanceChain` — call a parametric instance's ctor global with its
	///     inner dicts to materialize a fresh dict.
	fn lower_dict_atom(&mut self, resolved: &DispatchTarget, range: Range) -> Result<Atom, String> {
		match resolved {
			DispatchTarget::Global(slot_name) => {
				let (module, name) = slot_name
					.rsplit_once('.')
					.ok_or("malformed instance slot name")?;
				let gid = self
					.globals
					.lookup(module, name)
					.ok_or("instance slot not registered as a global")?;
				Ok(self.emit_let(Rvalue::GlobalRef(gid), range))
			}
			DispatchTarget::Forwarded(slot) => {
				let name = synthetic_dict_name(*slot);
				match self.resolve(&name)? {
					Resolved::Atom(a) => Ok(a),
					_ => Err(format!("dispatch slot `{}` resolved to a non-local", name)),
				}
			}
			DispatchTarget::InstanceChain { ctor_slot, inner } => {
				let (module, name) = ctor_slot
					.rsplit_once('.')
					.ok_or("malformed ctor slot name")?;
				let gid = self
					.globals
					.lookup(module, name)
					.ok_or("ctor slot not registered as a global")?;
				let ctor = self.emit_let(Rvalue::GlobalRef(gid), range);
				let mut args = Vec::with_capacity(inner.len());
				for r in inner {
					args.push(self.lower_dict_atom(r, range)?);
				}
				Ok(self.emit_let(Rvalue::CallClosure(ctor, args), range))
			}
		}
	}

	/// `left | right` — pipe `left` in as the first argument of the call on the
	/// right (or, if `right` isn't a call, call it with the single argument).
	fn lower_chain(
		&mut self,
		left: &ExprNode,
		right: &ExprNode,
		range: Range,
	) -> Result<Atom, String> {
		let (callee, extra): (&ExprNode, &[ExprNode]) = match &right.kind {
			ExprKind::Call(c) => {
				if !c.dict_args.is_empty() {
					return Err("trait-constrained call in a pipe not yet supported".to_string());
				}
				(c.callee.as_ref(), c.args.as_slice())
			}
			_ => (right, &[]),
		};
		// Evaluate callee, then the piped value, then the remaining args
		// (matching `emit.rs`'s ordering).
		let callee_atom = self.lower_expr(callee)?;
		let left_atom = self.lower_expr(left)?;
		let mut args = Vec::with_capacity(1 + extra.len());
		args.push(left_atom);
		for a in extra {
			args.push(self.lower_expr(a)?);
		}
		Ok(self.emit_let(Rvalue::CallClosure(callee_atom, args), range))
	}

	// ---- control flow ---------------------------------------------------

	/// `if subject is pattern { body } [else { else_body }]`. Lowers to a
	/// `Match`: the pattern arm, plus a wildcard `else` arm when present. The
	/// result lives in a fresh var (defaulting to `nothing` — which is exactly
	/// the value of an `else`-less `if`, since its body value is discarded).
	fn lower_if(&mut self, n: &IfNode, range: Range) -> Result<Atom, String> {
		let subject = self.lower_expr(&n.subject)?;
		let result = self.alloc_var();
		let has_else = n.else_body.is_some();

		let mark = self.cur().locals.len();
		let pattern = self.lower_pattern(&n.pattern, &n.subject.ty)?;
		let body = self.lower_block_of(&n.body, if has_else { Some(result) } else { None })?;
		self.cur().locals.truncate(mark);
		let mut arms = vec![MatchArm { pattern, body }];

		if let Some(else_body) = &n.else_body {
			let else_block = self.lower_block_of(else_body, Some(result))?;
			arms.push(MatchArm {
				pattern: Pattern::Wildcard,
				body: else_block,
			});
		}
		self.push_stmt(StmtKind::Match { subject, arms }, range);
		Ok(Atom::Var(result))
	}

	/// `when subject is p1 { b1 } is p2 { b2 } ...`. Each case is a match arm;
	/// the arm bodies all write the shared result var.
	fn lower_when(&mut self, n: &WhenNode, range: Range) -> Result<Atom, String> {
		let subject = self.lower_expr(&n.subject)?;
		let result = self.alloc_var();
		let mut arms = Vec::with_capacity(n.cases.len());
		for case in &n.cases {
			let mark = self.cur().locals.len();
			let pattern = self.lower_pattern(&case.pattern, &n.subject.ty)?;
			let body = self.lower_block_of(&case.body, Some(result))?;
			self.cur().locals.truncate(mark);
			arms.push(MatchArm { pattern, body });
		}
		self.push_stmt(StmtKind::Match { subject, arms }, range);
		Ok(Atom::Var(result))
	}

	/// `while subject is pattern { body }`. A `Loop` that re-evaluates the
	/// subject each iteration and matches it: on match, run the body and
	/// continue; otherwise break. Evaluates to `nothing`.
	fn lower_while(&mut self, n: &WhileNode, range: Range) -> Result<Atom, String> {
		let saved = self.take_stmts();
		let res = self.lower_while_body(n, range);
		let loop_stmts = self.restore_stmts(saved);
		res?;
		self.push_stmt(StmtKind::Loop(Block(loop_stmts)), range);
		Ok(Atom::Const(Const::Unit))
	}

	fn lower_while_body(&mut self, n: &WhileNode, range: Range) -> Result<(), String> {
		let subject = self.lower_expr(&n.subject)?;
		let mark = self.cur().locals.len();
		let pattern = self.lower_pattern(&n.pattern, &n.subject.ty)?;
		let mut matched = self.lower_block_of(&n.body, None)?;
		matched.0.push(Stmt::new(StmtKind::Continue, range));
		self.cur().locals.truncate(mark);
		let arms = vec![
			MatchArm {
				pattern,
				body: matched,
			},
			MatchArm {
				pattern: Pattern::Wildcard,
				body: Block(vec![Stmt::new(StmtKind::Break, range)]),
			},
		];
		self.push_stmt(StmtKind::Match { subject, arms }, range);
		Ok(())
	}

	/// Lower a body (sequence of statements) into its own `Block`, redirecting
	/// emitted statements into a fresh buffer. If `result` is `Some`, the
	/// body's last value is assigned to it; otherwise the body runs for effects.
	fn lower_block_of(&mut self, body: &[ExprNode], result: Option<VarId>) -> Result<Block, String> {
		let saved = self.take_stmts();
		let res = self.lower_stmts_into(body, result);
		let stmts = self.restore_stmts(saved);
		res?;
		Ok(Block(stmts))
	}

	fn lower_stmts_into(&mut self, body: &[ExprNode], result: Option<VarId>) -> Result<(), String> {
		// The block's result-binding `Let` lives at the block-trailing position.
		// Anchor it to the last expr's range (or `SYNTHETIC` for an empty body)
		// so any error attribution / `debug` call lands on the producing line.
		let trail_range = body.last().map(|e| e.range).unwrap_or(SYNTHETIC);
		let assign = |s: &mut Self, atom: Atom| {
			if let Some(r) = result {
				s.push_stmt(StmtKind::Let(r, Rvalue::Use(atom)), trail_range);
			}
		};
		if body.is_empty() {
			assign(self, Atom::Const(Const::Unit));
			return Ok(());
		}
		let last = body.len() - 1;
		for (i, e) in body.iter().enumerate() {
			if let ExprKind::Let(let_node) = &e.kind {
				self.lower_let(let_node)?;
				if i == last {
					assign(self, Atom::Const(Const::Unit));
				}
			} else {
				let atom = self.lower_expr(e)?;
				if i == last {
					assign(self, atom);
				}
			}
		}
		Ok(())
	}

	fn take_stmts(&mut self) -> Vec<Stmt> {
		std::mem::take(&mut self.cur().stmts)
	}

	fn restore_stmts(&mut self, saved: Vec<Stmt>) -> Vec<Stmt> {
		std::mem::replace(&mut self.cur().stmts, saved)
	}

	// ---- patterns -------------------------------------------------------

	fn lower_pattern(&mut self, pat: &PatternNode, subject_ty: &Type) -> Result<Pattern, String> {
		match &pat.kind {
			PatternKind::Underscore => Ok(Pattern::Wildcard),
			PatternKind::Identifier(id) => {
				// A bare identifier is a nullary-variant match when it names one
				// of the subject enum's nullary variants; `true`/`false` match a
				// bool; otherwise it's an irrefutable binding.
				if let Type::Enum(qualified, _) = subject_ty {
					let is_variant = self
						.enums
						.get(qualified)
						.map_or(false, |vs| vs.iter().any(|(n, a)| n == &id.name && *a == 0));
					if is_variant {
						return Ok(Pattern::Variant {
							variant: id.name.clone(),
							fields: Vec::new(),
						});
					}
				}
				if matches!(subject_ty, Type::Bool) && (id.name == "true" || id.name == "false") {
					return Ok(Pattern::Literal(Const::Bool(id.name == "true")));
				}
				let v = self.alloc_var();
				self.cur().locals.push((id.name.clone(), v));
				Ok(Pattern::Bind(v))
			}
			PatternKind::Literal(lit) => Ok(Pattern::Literal(literal_to_const(&lit.kind)?)),
			PatternKind::Constructor(variant, subs) => {
				let fields = self.lower_sub_patterns(subs)?;
				Ok(Pattern::Variant {
					variant: variant.name.clone(),
					fields,
				})
			}
			PatternKind::Tuple(elems) => Ok(Pattern::Tuple(self.lower_sub_patterns(elems)?)),
			PatternKind::List { items, rest } => {
				let items = self.lower_sub_patterns(items)?;
				let rest = match rest {
					None => None,
					Some(rp) => {
						Some(self.lower_rest_binding(rp.binding.as_ref(), ListRest::Anon, ListRest::Bind))
					}
				};
				Ok(Pattern::List { items, rest })
			}
			PatternKind::Record { fields, rest } => {
				let mut ir_fields = Vec::with_capacity(fields.len());
				for (name, p) in fields {
					// Sub-patterns carry no known subject type (matching `emit.rs`).
					ir_fields.push((name.name.clone(), self.lower_pattern(p, &Type::Unknown)?));
				}
				let rest = match rest {
					None => RecordRest::Exact,
					Some(rp) => {
						self.lower_rest_binding(rp.binding.as_ref(), RecordRest::Open, RecordRest::Bind)
					}
				};
				Ok(Pattern::Record {
					fields: ir_fields,
					rest,
				})
			}
			PatternKind::Interpolation(_) => {
				Err("string-interpolation pattern not yet supported".to_string())
			}
		}
	}

	/// Lower a list of sub-patterns. They carry no known subject type, so a
	/// bare identifier is always a binding (mirrors `emit.rs`, which passes
	/// `Type::Unknown` to sub-pattern emission).
	fn lower_sub_patterns(&mut self, subs: &[PatternNode]) -> Result<Vec<Pattern>, String> {
		let mut out = Vec::with_capacity(subs.len());
		for sub in subs {
			out.push(self.lower_pattern(sub, &Type::Unknown)?);
		}
		Ok(out)
	}

	/// Resolve a list/record rest binding: an anonymous `...` (no capture) or a
	/// `...name` that binds a fresh variable.
	fn lower_rest_binding<T>(
		&mut self,
		binding: Option<&compiler::ast::IdentifierNode>,
		anon: T,
		bind: impl FnOnce(VarId) -> T,
	) -> T {
		match binding {
			None => anon,
			Some(id) => {
				let v = self.alloc_var();
				self.cur().locals.push((id.name.clone(), v));
				bind(v)
			}
		}
	}

	fn lower_fun(&mut self, fun: &FunNode, range: Range) -> Result<Atom, String> {
		let param_names: Vec<&str> = fun.params.iter().map(|p| p.ident.name.as_str()).collect();
		let fn_name = format!(
			"{}.fun@{}:{}",
			self.current_module, fun.range.start.line, fun.range.start.col
		);
		self.lower_closure(fn_name, &param_names, &fun.body, range)
	}

	/// Lower a closure body into its own `Function` and return a `MakeClosure`
	/// atom for it. Shared by `fun` literals, `defer` thunks, and `scope` body
	/// closures. A task `try` anywhere in the body marks the new function
	/// `is_async` (via `lower_try`), which drives `MakeAsyncClosure` in the
	/// emitter — mirroring `emit.rs`'s `body_is_async` decision, but observed
	/// during lowering rather than pre-scanned.
	fn lower_closure(
		&mut self,
		fn_name: String,
		param_names: &[&str],
		body: &[ExprNode],
		outer_range: Range,
	) -> Result<Atom, String> {
		self.push_scope(fn_name, param_names);
		let body_range = body.last().map(|e| e.range).unwrap_or(outer_range);
		if let Err(e) = self.lower_body_tail(body, body_range) {
			self.scopes.pop();
			return Err(e);
		}
		let scope = self.scopes.pop().unwrap();

		// Build the closure's capture list (resolved against the now-current
		// parent scope) before consuming `scope`.
		let capture_atoms: Vec<Atom> = scope
			.captures
			.iter()
			.map(|c| self.capture_src_atom(&c.src))
			.collect();
		let fid = self.add_function(finish_scope(scope));
		Ok(self.emit_let(Rvalue::MakeClosure(fid, capture_atoms), outer_range))
	}

	/// `defer expr` — build a zero-arg cleanup closure `fun { expr }` and push
	/// it onto the running frame's cleanup stack. The defer expression itself
	/// evaluates to `nothing`. The VM walks the stack LIFO at `Return` (and on
	/// `try`-failure short-circuit). Mirrors `codegen::emit`'s `ExprKind::Defer`
	/// arm.
	fn lower_defer(&mut self, inner: &ExprNode, range: Range) -> Result<Atom, String> {
		let fn_name = format!(
			"{}.defer@{}:{}",
			self.current_module, inner.range.start.line, inner.range.start.col
		);
		let closure = self.lower_closure(fn_name, &[], std::slice::from_ref(inner), range)?;
		self.push_stmt(StmtKind::PushDefer(closure), range);
		Ok(Atom::Const(Const::Unit))
	}

	/// A task-carrier `try Pattern = value` and its continuation (`rest`). Lowers
	/// to: evaluate the awaited task, `Await` it (suspend), bind the pattern, then
	/// lower the continuation inline — the CPS state machine, mirroring
	/// `emit.rs`'s `Try` arm. Sets the enclosing function `is_async` (its frame
	/// awaits), which drives `MakeAsyncClosure`. `option`/`result` `try`s are
	/// rewritten to `<carrier>.then` calls by the analyzer and never reach here.
	fn lower_try(&mut self, node: &TryNode, range: Range) -> Result<Atom, String> {
		if !node.task_carrier {
			return Err("non-task `try` was not rewritten by the analyzer".to_string());
		}
		self.cur().is_async = true;
		let task_atom = self.lower_expr(&node.value)?;
		match &node.pattern.kind {
			PatternKind::Identifier(id) => {
				let v = self.alloc_var();
				self.push_stmt(StmtKind::Let(v, Rvalue::Await(task_atom)), range);
				self.cur().locals.push((id.name.clone(), v));
			}
			PatternKind::Underscore => {
				self.push_stmt(StmtKind::Discard(Rvalue::Await(task_atom)), range);
			}
			// The analyzer restricts a task `try` pattern to ident/wildcard.
			_ => return Err("unsupported task `try` pattern".to_string()),
		}
		// The continuation: its last expr is the chain's (and the function's)
		// tail task. Bindings introduced above stay in scope for it.
		self.lower_body(&node.rest)
	}

	/// `scope (as s)? { body }` / `manual scope ...` — lower to a call to the
	/// `core.task.scope-new` kernel: `scope-new <manual> (fun handle { body })`.
	/// The body becomes its own closure frame (so its `try`s suspend within the
	/// scope's child fiber, not this one — that's why a `scope` doesn't make the
	/// enclosing function async). Mirrors `emit.rs`'s `emit_scope`.
	fn lower_scope(&mut self, node: &ScopeNode, range: Range) -> Result<Atom, String> {
		let g = self
			.globals
			.lookup("core.task", "scope-new")
			.ok_or("`core.task.scope-new` not found")?;
		let scope_new = self.emit_let(Rvalue::GlobalRef(g), range);
		let manual = Atom::Const(Const::Bool(node.manual));
		// The body closure's parameter carries the `scope as NAME` handle so the
		// body's `s.*` references resolve to it; an anonymous scope gets an
		// unreferenced synthetic parameter.
		let handle_name = node.handle.as_ref().map_or("__scope", |h| h.name.as_str());
		let fn_name = format!(
			"{}.scope@{}:{}",
			self.current_module, range.start.line, range.start.col
		);
		let body = self.lower_closure(fn_name, &[handle_name], &node.body, range)?;
		Ok(self.emit_let(Rvalue::CallClosure(scope_new, vec![manual, body]), range))
	}

	/// Lower a function/`let`-block body (a sequence of statements). Returns
	/// the value the body evaluates to (its last expression). Used where the
	/// body's value flows *into* something (e.g. a `try` continuation), as
	/// opposed to `lower_body_tail` which returns it directly.
	fn lower_body(&mut self, body: &[ExprNode]) -> Result<Atom, String> {
		if body.is_empty() {
			return Ok(Atom::Const(Const::Unit));
		}
		let last = body.len() - 1;
		for (i, e) in body.iter().enumerate() {
			if let ExprKind::Let(let_node) = &e.kind {
				self.lower_let(let_node)?;
				if i == last {
					return Ok(Atom::Const(Const::Unit));
				}
			} else {
				let atom = self.lower_expr(e)?;
				if i == last {
					return Ok(atom);
				}
				// Non-last expression: its effects are already emitted as
				// `Let`-bound rvalues; the unused result atom just falls away.
			}
		}
		Ok(Atom::Const(Const::Unit))
	}

	// ---- tail position --------------------------------------------------
	//
	// A function body's last expression is in *tail position*: its value is the
	// function's return value. Lowering it through the tail path emits the
	// `Return` (and, for a direct call, a `TailCall`) itself, threading tail-ness
	// into `when`/`if` arms so the recursive call sitting in an arm becomes a
	// real tail call — mirroring `emit.rs`'s `tail: bool`. Everything that isn't
	// a `when`/`if`/direct-call simply evaluates to an atom and `Return`s it.

	/// Lower a body whose final value is the enclosing function's return,
	/// emitting the `Return` directly. Non-final statements run for effect.
	fn lower_body_tail(&mut self, body: &[ExprNode], body_range: Range) -> Result<(), String> {
		if body.is_empty() {
			self.push_stmt(StmtKind::Return(Atom::Const(Const::Unit)), body_range);
			return Ok(());
		}
		let last = body.len() - 1;
		for (i, e) in body.iter().enumerate() {
			if i < last {
				if let ExprKind::Let(let_node) = &e.kind {
					self.lower_let(let_node)?;
				} else {
					self.lower_expr(e)?;
				}
			} else if let ExprKind::Let(let_node) = &e.kind {
				// A block ending in a `let` evaluates to `nothing`.
				self.lower_let(let_node)?;
				self.push_stmt(StmtKind::Return(Atom::Const(Const::Unit)), body_range);
			} else {
				self.lower_tail(e)?;
			}
		}
		Ok(())
	}

	/// Lower one expression in tail position. `when`/`if`/direct-call get tail
	/// treatment; anything else evaluates to an atom and is `Return`ed.
	fn lower_tail(&mut self, expr: &ExprNode) -> Result<(), String> {
		let range = expr.range;
		match &expr.kind {
			ExprKind::Grouping(inner) => self.lower_tail(inner),
			ExprKind::When(n) => self.lower_when_tail(n, range),
			ExprKind::If(n) => self.lower_if_tail(n, range),
			// A direct call in tail position is a tail call — but only an
			// ordinary (non-dispatched) call; a trait-constrained call still
			// carries `dict_args` and is handled fine by the generic path below,
			// just without TCO (its dicts ride as leading args, same as a normal
			// call, so it's eligible too).
			ExprKind::Call(call) => self.lower_call_tail(call, range),
			_ => {
				let atom = self.lower_expr(expr)?;
				self.push_stmt(StmtKind::Return(atom), range);
				Ok(())
			}
		}
	}

	/// Lower a sub-block (a `when`/`if` arm body) in tail position into its own
	/// `Block`, redirecting emitted statements into a fresh buffer.
	fn lower_tail_block(&mut self, body: &[ExprNode], range: Range) -> Result<Block, String> {
		let saved = self.take_stmts();
		let res = self.lower_body_tail(body, range);
		let stmts = self.restore_stmts(saved);
		res?;
		Ok(Block(stmts))
	}

	/// `when` in tail position: each arm `Return`s its value directly (no shared
	/// result var). A subject that matches no arm falls through to `Return
	/// nothing` — matching the non-tail `when`'s `nothing` default.
	fn lower_when_tail(&mut self, n: &WhenNode, range: Range) -> Result<(), String> {
		let subject = self.lower_expr(&n.subject)?;
		let mut arms = Vec::with_capacity(n.cases.len());
		for case in &n.cases {
			let mark = self.cur().locals.len();
			let pattern = self.lower_pattern(&case.pattern, &n.subject.ty)?;
			let body_range = case.body.last().map(|e| e.range).unwrap_or(range);
			let body = self.lower_tail_block(&case.body, body_range)?;
			self.cur().locals.truncate(mark);
			arms.push(MatchArm { pattern, body });
		}
		self.push_stmt(StmtKind::Match { subject, arms }, range);
		self.push_stmt(StmtKind::Return(Atom::Const(Const::Unit)), range);
		Ok(())
	}

	/// `if` in tail position: the matching arm (and the `else`, if present)
	/// `Return` directly; a no-match falls through to `Return nothing`.
	fn lower_if_tail(&mut self, n: &IfNode, range: Range) -> Result<(), String> {
		let subject = self.lower_expr(&n.subject)?;
		let mark = self.cur().locals.len();
		let pattern = self.lower_pattern(&n.pattern, &n.subject.ty)?;
		let then_range = n.body.last().map(|e| e.range).unwrap_or(range);
		let then_block = self.lower_tail_block(&n.body, then_range)?;
		self.cur().locals.truncate(mark);
		let mut arms = vec![MatchArm {
			pattern,
			body: then_block,
		}];
		if let Some(else_body) = &n.else_body {
			let else_range = else_body.last().map(|e| e.range).unwrap_or(range);
			let else_block = self.lower_tail_block(else_body, else_range)?;
			arms.push(MatchArm {
				pattern: Pattern::Wildcard,
				body: else_block,
			});
		}
		self.push_stmt(StmtKind::Match { subject, arms }, range);
		self.push_stmt(StmtKind::Return(Atom::Const(Const::Unit)), range);
		Ok(())
	}

	/// A direct call in tail position: lower like `lower_call` but emit a
	/// `TailCall` and `Return` its result. The trailing `Return` is dead for a
	/// closure callee (the VM reuses the frame) and live for a
	/// builtin/ctor/async-fn callee (which ignores the tail flag).
	fn lower_call_tail(
		&mut self,
		call: &compiler::ast::CallNode,
		range: Range,
	) -> Result<(), String> {
		let callee = self.lower_expr(&call.callee)?;
		let mut args = Vec::with_capacity(call.dict_args.len() + call.args.len());
		for cell in &call.dict_args {
			args.push(self.lower_dispatch(cell, range)?);
		}
		for a in &call.args {
			args.push(self.lower_expr(a)?);
		}
		let v = self.alloc_var();
		self.push_stmt(StmtKind::Let(v, Rvalue::TailCall(callee, args)), range);
		self.push_stmt(StmtKind::Return(Atom::Var(v)), range);
		Ok(())
	}

	fn lower_let(&mut self, let_node: &LetNode) -> Result<(), String> {
		let value_range = let_node.value.range;
		match &let_node.pattern.kind {
			PatternKind::Identifier(id) => {
				let atom = self.lower_expr(&let_node.value)?;
				let var = match atom {
					Atom::Var(v) => v,
					other => {
						let v = self.alloc_var();
						self.push_stmt(StmtKind::Let(v, Rvalue::Use(other)), value_range);
						v
					}
				};
				self.cur().locals.push((id.name.clone(), var));
				Ok(())
			}
			PatternKind::Underscore => {
				// `let _ = e` — evaluate `e` for its effect and discard the value.
				self.lower_expr(&let_node.value)?;
				Ok(())
			}
			_ => {
				// Irrefutable destructuring (`let (a, b) = …`, `let {x, y} = …`):
				// the analyzer guarantees the pattern can't fail, so a single-arm
				// `Match` binds the parts. Unlike `when`/`if`, the bindings stay in
				// scope for the rest of the block (no `locals` truncation).
				let subject = self.lower_expr(&let_node.value)?;
				let pattern = self.lower_pattern(&let_node.pattern, &let_node.value.ty)?;
				self.push_stmt(
					StmtKind::Match {
						subject,
						arms: vec![MatchArm {
							pattern,
							body: Block(Vec::new()),
						}],
					},
					let_node.range,
				);
				Ok(())
			}
		}
	}

	// ---- scopes / name resolution --------------------------------------

	fn cur(&mut self) -> &mut FnScope {
		self.scopes.last_mut().expect("a scope is active")
	}

	fn push_scope(&mut self, name: String, param_names: &[&str]) {
		let mut scope = FnScope {
			name,
			module: self.current_module.clone(),
			params: Vec::new(),
			captures: Vec::new(),
			locals: Vec::new(),
			next_var: 0,
			stmts: Vec::new(),
			is_async: false,
		};
		for pn in param_names {
			let v = VarId(scope.next_var);
			scope.next_var += 1;
			scope.params.push(v);
			scope.locals.push((pn.to_string(), v));
		}
		self.scopes.push(scope);
	}

	fn fresh_var(&mut self, scope_idx: usize) -> VarId {
		let s = &mut self.scopes[scope_idx];
		let v = VarId(s.next_var);
		s.next_var += 1;
		v
	}

	fn alloc_var(&mut self) -> VarId {
		let idx = self.scopes.len() - 1;
		self.fresh_var(idx)
	}

	fn emit_let(&mut self, rv: Rvalue, range: Range) -> Atom {
		let v = self.alloc_var();
		self
			.cur()
			.stmts
			.push(Stmt::new(StmtKind::Let(v, rv), range));
		Atom::Var(v)
	}

	/// Push a stmt with no source-level origin (entry/poison thunks, dict
	/// scaffolding, internal `Use` re-bindings).
	fn push_synthetic(&mut self, kind: StmtKind) {
		self.cur().stmts.push(Stmt::synthetic(kind));
	}

	/// Push a stmt anchored at `range`.
	fn push_stmt(&mut self, kind: StmtKind, range: Range) {
		self.cur().stmts.push(Stmt::new(kind, range));
	}

	fn capture_src_atom(&self, src: &CaptureSrc) -> Atom {
		match src {
			CaptureSrc::ParentLocal(v) => Atom::Var(*v),
			CaptureSrc::ParentCapture(i) => {
				Atom::Var(self.scopes.last().expect("parent scope").captures[*i].var)
			}
		}
	}

	fn resolve(&mut self, name: &str) -> Result<Resolved, String> {
		let top = self.scopes.len() - 1;
		match self.resolve_at(top, name) {
			Some(ScopeSlot::Local(v)) => Ok(Resolved::Atom(Atom::Var(v))),
			Some(ScopeSlot::Capture(i)) => {
				let var = self.scopes[top].captures[i].var;
				Ok(Resolved::Atom(Atom::Var(var)))
			}
			Some(ScopeSlot::Global(g)) => Ok(Resolved::Global(g)),
			Some(ScopeSlot::BareVariant {
				qualified,
				variant,
				arity,
			}) => Ok(Resolved::BareVariant {
				qualified,
				variant,
				arity,
			}),
			None => Err(format!("unbound identifier `{}`", name)),
		}
	}

	/// Resolve `name` as seen from scope `scope_idx`, capturing through parents
	/// as needed. Mirrors `codegen::emit::resolve_identifier`.
	fn resolve_at(&mut self, scope_idx: usize, name: &str) -> Option<ScopeSlot> {
		if let Some(v) = self.scopes[scope_idx]
			.locals
			.iter()
			.rev()
			.find(|(n, _)| n == name)
			.map(|(_, v)| *v)
		{
			return Some(ScopeSlot::Local(v));
		}
		if let Some(i) = self.scopes[scope_idx]
			.captures
			.iter()
			.position(|c| c.name == name)
		{
			return Some(ScopeSlot::Capture(i));
		}
		if scope_idx > 0 {
			match self.resolve_at(scope_idx - 1, name) {
				Some(ScopeSlot::Local(pv)) => {
					return Some(self.add_capture(scope_idx, name, CaptureSrc::ParentLocal(pv)))
				}
				Some(ScopeSlot::Capture(pi)) => {
					return Some(self.add_capture(scope_idx, name, CaptureSrc::ParentCapture(pi)))
				}
				// Globals and bare variants are loaded directly at the use site,
				// never captured.
				Some(other) => return Some(other),
				None => {}
			}
		}
		// Module-level: same-module global, then prelude global, then a bare
		// variant constructor.
		if let Some(g) = self.globals.lookup(&self.current_module, name) {
			return Some(ScopeSlot::Global(g));
		}
		if let Some(g) = self.globals.lookup("__prelude__", name) {
			return Some(ScopeSlot::Global(g));
		}
		self.lookup_bare_variant(name)
	}

	fn add_capture(&mut self, scope_idx: usize, name: &str, src: CaptureSrc) -> ScopeSlot {
		let var = self.fresh_var(scope_idx);
		let i = self.scopes[scope_idx].captures.len();
		self.scopes[scope_idx].captures.push(CaptureInfo {
			name: name.to_string(),
			var,
			src,
		});
		ScopeSlot::Capture(i)
	}

	fn lookup_bare_variant(&self, name: &str) -> Option<ScopeSlot> {
		// Local-module enums win over imported/prelude variants of the same
		// name (mirrors the analyzer's disambiguation).
		let local_prefix = format!("{}.", self.current_module);
		let mut local = None;
		let mut other = None;
		for (qualified, variants) in &self.enums {
			for (variant, arity) in variants {
				if variant == name {
					let slot = ScopeSlot::BareVariant {
						qualified: qualified.clone(),
						variant: variant.clone(),
						arity: *arity,
					};
					if qualified.starts_with(&local_prefix) {
						local = Some(slot);
					} else if other.is_none() {
						other = Some(slot);
					}
				}
			}
		}
		local.or(other)
	}

	// ---- entry / poison / function table -------------------------------

	fn build_entry(&mut self, has_tests: bool) -> Result<FuncId, String> {
		let main_module = self
			.compiler
			.entry_modules
			.first()
			.ok_or("no entry module")?
			.clone();
		let main = match self.globals.lookup(&main_module, "main") {
			Some(g) => g,
			// No `main`, but `pluma test` programs are entered via a no-op (push
			// `nothing` and return); the runner then invokes each test directly.
			None if has_tests => {
				let func = Function {
					name: "__entry__".to_string(),
					module: String::new(),
					params: Vec::new(),
					captures: Vec::new(),
					is_async: false,
					body: Block(vec![Stmt::synthetic(StmtKind::Return(Atom::Const(
						Const::Unit,
					)))]),
					var_reprs: Vec::new(),
				};
				return Ok(self.add_function(func));
			}
			None => return Err(format!("module `{}` has no `main` def", main_module)),
		};
		// Load `main`, call it with the unit arg, return the result.
		let func = Function {
			name: "__entry__".to_string(),
			module: String::new(),
			params: Vec::new(),
			captures: Vec::new(),
			is_async: false,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(VarId(0), Rvalue::GlobalRef(main))),
				Stmt::synthetic(StmtKind::Let(
					VarId(1),
					Rvalue::CallClosure(Atom::Var(VarId(0)), vec![Atom::Const(Const::Unit)]),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(VarId(1)))),
			]),
			var_reprs: Vec::new(),
		};
		Ok(self.add_function(func))
	}

	/// Point a global at the shared poison thunk — used for any def whose body
	/// uses a not-yet-supported construct. Running it returns `nothing`; a
	/// fixture that never reaches it is unaffected, while one that does will
	/// diverge from the reference output (flagging the gap).
	fn poison_global(&mut self, gid: GlobalId) {
		let fid = self.poison_fn();
		self.globals.set_thunk(gid, fid);
	}

	fn poison_fn(&mut self) -> FuncId {
		if let Some(f) = self.poison {
			return f;
		}
		let func = Function {
			name: "__poison__".to_string(),
			module: String::new(),
			params: Vec::new(),
			captures: Vec::new(),
			is_async: false,
			body: Block(vec![Stmt::synthetic(StmtKind::Return(Atom::Const(
				Const::Unit,
			)))]),
			var_reprs: Vec::new(),
		};
		let f = self.add_function(func);
		self.poison = Some(f);
		f
	}

	fn add_function(&mut self, f: Function) -> FuncId {
		let id = FuncId(self.functions.len() as u32);
		self.functions.push(f);
		id
	}
}

fn finish_scope(scope: FnScope) -> Function {
	Function {
		name: scope.name,
		module: scope.module,
		params: scope.params,
		captures: scope.captures.iter().map(|c| c.var).collect(),
		is_async: scope.is_async,
		body: Block(scope.stmts),
		// Filled in by a single pass over all functions at the end of `run`.
		var_reprs: Vec::new(),
	}
}

/// Build a regex pattern string from a regex-literal AST node. Ported from
/// `codegen::emit::regex_pattern`; the analyzer has already validated the node,
/// so the resulting pattern compiles.
fn regex_pattern(node: &RegexNode) -> String {
	match &node.kind {
		RegexKind::Literal(s) => regex::escape(s),
		RegexKind::CharacterClass(c) => match c.as_str() {
			"any" => ".".to_string(),
			"digit" => "[0-9]".to_string(),
			"letter" => "[A-Za-z]".to_string(),
			"whitespace" => "[ \\t\\n\\r]".to_string(),
			"word" => "[A-Za-z0-9_]".to_string(),
			_ => "[^\\s\\S]".to_string(),
		},
		RegexKind::Anchor(a) => match a {
			RegexAnchor::Start => "^".to_string(),
			RegexAnchor::End => "$".to_string(),
			RegexAnchor::Boundary => "\\b".to_string(),
		},
		RegexKind::OneOrMore(inner) => format!("(?:{})+", regex_pattern(inner)),
		RegexKind::ZeroOrMore(inner) => format!("(?:{})*", regex_pattern(inner)),
		RegexKind::OneOrZero(inner) => format!("(?:{})?", regex_pattern(inner)),
		RegexKind::ExactCount(inner, n) => format!("(?:{}){{{}}}", regex_pattern(inner), n),
		RegexKind::AtLeastCount(inner, n) => format!("(?:{}){{{},}}", regex_pattern(inner), n),
		RegexKind::AtMostCount(inner, n) => format!("(?:{}){{0,{}}}", regex_pattern(inner), n),
		RegexKind::RangeCount(inner, min, max) => {
			format!("(?:{}){{{},{}}}", regex_pattern(inner), min, max)
		}
		RegexKind::Grouping(inner) => format!("(?:{})", regex_pattern(inner)),
		RegexKind::Sequence(parts) => parts.iter().map(regex_pattern).collect(),
		RegexKind::Alternation(parts) => {
			let joined: Vec<_> = parts.iter().map(regex_pattern).collect();
			format!("(?:{})", joined.join("|"))
		}
		RegexKind::NamedCapture(name, inner) => {
			format!("(?P<{}>{})", name, regex_pattern(inner))
		}
	}
}

/// Build the module's local-namespace -> qualified-module map: explicit `use`
/// declarations plus the auto-imported modules (unless shadowed). Mirrors
/// `codegen::emit::compile_module`.
fn build_imports(ast: &ModuleNode) -> HashMap<String, String> {
	let mut imports: HashMap<String, String> = ast
		.uses
		.iter()
		.map(|u| (u.local_name().name.clone(), u.module_name()))
		.collect();
	for (full, local) in compiler::AUTO_IMPORTS {
		imports
			.entry(local.to_string())
			.or_insert_with(|| full.to_string());
	}
	imports
}

/// If `expr` carries a non-empty, undrained dispatch sink, return its cells. A
/// surviving sink means a trait-constrained value referenced in value position
/// (passed, returned, or bound — not directly called), which needs its dicts
/// pre-applied (`lower_constrained_value_ref`). An empty sink is treated as
/// absent. Mirrors `codegen::emit::undrained_dispatch_cells`.
fn undrained_dispatch_cells(expr: &ExprNode) -> Option<Vec<compiler::ast::DispatchCell>> {
	let sink = expr.dispatch_sink.as_ref()?;
	let cells = sink.borrow();
	if cells.is_empty() {
		None
	} else {
		Some(cells.iter().cloned().collect())
	}
}

/// The synthetic local name a constrained def / instance ctor binds its hidden
/// dict parameter `slot` under, so `Forwarded` dispatch resolves by name (and
/// captures through nested closures). Mirrors `codegen::emit::synthetic_dict_name`.
fn synthetic_dict_name(slot: u16) -> String {
	format!("__dict_{}__", slot)
}

/// If both operands of a `numeric`-dispatched arithmetic operator are the *same
/// concrete* numeric type (`int` or `float`), return the direct `BinOp` so the
/// dispatch can be devirtualized to a VM opcode. Returns `None` when either
/// operand is still a type variable (polymorphic — keep dispatching through the
/// runtime dict) or the two disagree (can't happen post-unification, but stays
/// honest). `%` never reaches the dispatched path (it carries no cell), so it's
/// already direct.
fn concrete_numeric_binop(op: &Operator, left: &Type, right: &Type) -> Option<BinOp> {
	let is_float = match (left, right) {
		(Type::Int, Type::Int) => false,
		(Type::Float, Type::Float) => true,
		_ => return None,
	};
	binop_for(op, is_float)
}

/// If a `< <= > >=` comparison has concrete numeric operands, return the direct
/// `BinOp` (`LtI64`/`LeF64`/…) so it lowers to the VM's relational opcode rather
/// than the `ord.compare … {==,!=} variant` desugaring. For concrete floats this
/// is the IEEE-754 comparison — `NaN` compares `false` for all four relations —
/// which is the language's defined semantics for concrete float relational
/// operators (consistent with structural `==`/`!=`, also IEEE, and deliberately
/// distinct from the total-order `ord.compare` that `list.sort` and generic `ord`
/// code use). Polymorphic operands return `None` (keep dispatching through the
/// runtime dict, where `ord`'s total order applies).
fn concrete_ord_binop(op: &Operator, left: &Type, right: &Type) -> Option<BinOp> {
	let is_float = match (left, right) {
		(Type::Int, Type::Int) => false,
		(Type::Float, Type::Float) => true,
		_ => return None,
	};
	binop_for(op, is_float)
}

/// Map a concrete (non-dispatched) operator to its IR `BinOp`. `is_float`
/// selects the arithmetic opcode variant. Returns `None` for operators that
/// aren't strict binary ops here (handled elsewhere or unsupported).
fn binop_for(op: &Operator, is_float: bool) -> Option<BinOp> {
	Some(match (op, is_float) {
		(Operator::Addition, false) => BinOp::AddInt,
		(Operator::Addition, true) => BinOp::AddFloat,
		(Operator::SubtractionOrNegation, false) => BinOp::SubInt,
		(Operator::SubtractionOrNegation, true) => BinOp::SubFloat,
		(Operator::Multiplication, false) => BinOp::MulInt,
		(Operator::Multiplication, true) => BinOp::MulFloat,
		(Operator::Division, false) => BinOp::DivInt,
		(Operator::Division, true) => BinOp::DivFloat,
		(Operator::Remainder, false) => BinOp::RemInt,
		(Operator::Remainder, true) => BinOp::RemFloat,
		(Operator::Concat, _) => BinOp::Concat,
		(Operator::LogicalAnd, _) => BinOp::And,
		(Operator::LogicalOr, _) => BinOp::Or,
		(Operator::Equality, _) => BinOp::Eq,
		(Operator::Inequality, _) => BinOp::Ne,
		// Ordering comparisons split by operand repr (see `BinOp`); reached only
		// for concrete numeric operands (comparisons otherwise dispatch through
		// `ord`), so `is_float` is authoritative.
		(Operator::LessThan, false) => BinOp::LtI64,
		(Operator::LessThan, true) => BinOp::LtF64,
		(Operator::LessThanEquals, false) => BinOp::LeI64,
		(Operator::LessThanEquals, true) => BinOp::LeF64,
		(Operator::GreaterThan, false) => BinOp::GtI64,
		(Operator::GreaterThan, true) => BinOp::GtF64,
		(Operator::GreaterThanEquals, false) => BinOp::GeI64,
		(Operator::GreaterThanEquals, true) => BinOp::GeF64,
		_ => return None,
	})
}

fn literal_to_const(kind: &LiteralKind) -> Result<Const, String> {
	Ok(match kind {
		LiteralKind::Bool(b) => Const::Bool(*b),
		LiteralKind::String(s) => Const::Str(s.clone()),
		LiteralKind::Bytes(b) => Const::Bytes(b.clone()),
		LiteralKind::FloatDecimal(f) => Const::Float(*f),
		LiteralKind::IntDecimal(n)
		| LiteralKind::IntHex(n)
		| LiteralKind::IntOctal(n)
		| LiteralKind::IntBinary(n) => Const::Int(*n as i64),
		LiteralKind::Duration(n) => Const::Duration(*n),
	})
}

fn expr_kind_name(kind: &ExprKind) -> &'static str {
	match kind {
		ExprKind::BinaryOperation { .. } => "binary operation",
		ExprKind::UnaryOperation { .. } => "unary operation",
		ExprKind::ElementAccess { .. } => "element access",
		ExprKind::FieldAccess { .. } => "field access",
		ExprKind::NamespaceAccess(_) => "namespace access",
		ExprKind::Fun(_) => "fun",
		ExprKind::Call(_) => "call",
		ExprKind::EmptyTuple => "empty tuple",
		ExprKind::Grouping(_) => "grouping",
		ExprKind::Identifier(_) => "identifier",
		ExprKind::Interpolation(_) => "interpolation",
		ExprKind::Let(_) => "let",
		ExprKind::Defer(_) => "defer",
		ExprKind::Literal(_) => "literal",
		ExprKind::Record(_) => "record",
		ExprKind::Tuple(_) => "tuple",
		ExprKind::Regex(_) => "regex",
		ExprKind::Try(_) => "try",
		ExprKind::Builtin(_) => "built-in",
		ExprKind::List(_) => "list",
		ExprKind::If(_) => "if",
		ExprKind::When(_) => "when",
		ExprKind::While(_) => "while",
		_ => "expression",
	}
}

// --------------------------------------------------------------------------
// Pre-pass: enum table.
// --------------------------------------------------------------------------

/// Collect every loaded module's enum definitions into the qualified-name ->
/// variants table. Mirrors `codegen::emit::collect_enum_defs`, run over all
/// modules (including the prelude, which defines `option`/`result`/`ordering`).
fn collect_enums(compiler: &Compiler) -> HashMap<String, Vec<(String, usize)>> {
	let mut out = HashMap::new();
	for (module_name, module) in &compiler.modules {
		let Some(ast) = &module.ast else { continue };
		for def in &ast.body {
			if let DefinitionKind::Enum(enum_node) = &def.kind {
				let qualified = format!("{}.{}", module_name, def.name.name);
				let variants = enum_node
					.variants
					.iter()
					.map(|v| {
						(
							v.name.name.clone(),
							v.params.as_ref().map_or(0, |p| p.len()),
						)
					})
					.collect();
				out.insert(qualified, variants);
			}
		}
	}
	out
}

// --------------------------------------------------------------------------
// Pre-pass: global-slot reservation.
// --------------------------------------------------------------------------

/// Lowering-internal global-slot table. Assigns a `GlobalId` per global in
/// allocation order and tracks each slot's initializer. Prelude/native slots
/// are pre-evaluated up front; user-def slots are `Reserved` until the expr
/// walk fills in their thunk `FuncId`. Becomes `IrProgram::globals` via
/// `finish` at the end of lowering.
struct GlobalTable {
	lookup: HashMap<(String, String), GlobalId>,
	slots: Vec<Slot>,
}

enum Slot {
	PreEvaluated(PreEval),
	/// A user def whose thunk function hasn't been emitted yet.
	Reserved,
	Thunk(FuncId),
}

impl GlobalTable {
	fn new() -> Self {
		Self {
			lookup: HashMap::new(),
			slots: Vec::new(),
		}
	}

	/// Reserve (or return the existing) slot for `(module, name)`. New slots
	/// start `Reserved`. Mirrors `codegen::emit::reserve_global`.
	fn reserve(&mut self, module: &str, name: &str) -> GlobalId {
		let key = (module.to_string(), name.to_string());
		if let Some(&id) = self.lookup.get(&key) {
			return id;
		}
		let id = GlobalId(self.slots.len() as u32);
		self.slots.push(Slot::Reserved);
		self.lookup.insert(key, id);
		id
	}

	/// Reserve a slot and fill it with a pre-evaluated value.
	fn add_pre_evaluated(&mut self, module: &str, name: &str, value: PreEval) -> GlobalId {
		let id = self.reserve(module, name);
		self.slots[id.0 as usize] = Slot::PreEvaluated(value);
		id
	}

	/// Fill an already-reserved slot with a pre-evaluated value (e.g. a
	/// `built-in "tag"` def).
	fn set_pre_evaluated(&mut self, id: GlobalId, value: PreEval) {
		self.slots[id.0 as usize] = Slot::PreEvaluated(value);
	}

	fn lookup(&self, module: &str, name: &str) -> Option<GlobalId> {
		self
			.lookup
			.get(&(module.to_string(), name.to_string()))
			.copied()
	}

	fn set_thunk(&mut self, id: GlobalId, func: FuncId) {
		self.slots[id.0 as usize] = Slot::Thunk(func);
	}

	fn finish(self) -> Vec<GlobalInit> {
		self
			.slots
			.into_iter()
			.map(|slot| match slot {
				Slot::PreEvaluated(v) => GlobalInit::PreEvaluated(v),
				Slot::Thunk(f) => GlobalInit::Thunk(f),
				Slot::Reserved => {
					panic!("global slot left reserved — a def thunk was never assigned")
				}
			})
			.collect()
	}
}

/// Seed the prelude's pre-evaluated globals: the `print`/`debug`/`to-string`
/// builtins and the concrete trait-instance method dictionaries. The dict
/// method order matches each trait's declaration order. Mirrors the prelude
/// block at the top of `codegen::emit::compile`, translated from `vm::Value`
/// into the vm-independent `PreEval`.
fn seed_prelude_globals(g: &mut GlobalTable) {
	let builtin = |tag: &str| PreEval::Builtin(tag.to_string());
	let dict = |tags: &[&str]| PreEval::MethodDict(tags.iter().map(|t| builtin(t)).collect());

	g.add_pre_evaluated("__prelude__", "print", builtin("print"));
	g.add_pre_evaluated("__prelude__", "debug", builtin("debug"));
	g.add_pre_evaluated("__prelude__", "to-string", builtin("to-string"));

	// `numeric`: add, sub, mul, div, negate.
	g.add_pre_evaluated(
		"__prelude__",
		"numeric@int",
		dict(&["int-add", "int-sub", "int-mul", "int-div", "int-negate"]),
	);
	g.add_pre_evaluated(
		"__prelude__",
		"numeric@float",
		dict(&[
			"float-add",
			"float-sub",
			"float-mul",
			"float-div",
			"float-negate",
		]),
	);

	// `ord`: compare.
	g.add_pre_evaluated("__prelude__", "ord@int", dict(&["int-compare"]));
	g.add_pre_evaluated("__prelude__", "ord@float", dict(&["float-compare"]));
	g.add_pre_evaluated("__prelude__", "ord@string", dict(&["string-compare"]));
	g.add_pre_evaluated("__prelude__", "ord@bytes", dict(&["bytes-compare"]));

	// `hash`: hash.
	g.add_pre_evaluated("__prelude__", "hash@int", dict(&["int-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@float", dict(&["float-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@string", dict(&["string-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@bytes", dict(&["bytes-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@bool", dict(&["bool-hash"]));
}

/// Reserve a slot for each user-module top-level value def, alias (its
/// constructor), and trait instance (its method dictionary). Enums and trait
/// declarations are types, not values, so they get no slot. Mirrors the
/// first reservation pass in `codegen::emit::compile`.
fn reserve_user_globals(g: &mut GlobalTable, compiler: &Compiler) {
	for (module_name, module) in &compiler.modules {
		let Some(ast) = &module.ast else { continue };
		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(_) | DefinitionKind::Alias(_) => {
					g.reserve(module_name, &def.name.name);
				}
				DefinitionKind::Enum(_) | DefinitionKind::Trait(_) => {}
				DefinitionKind::Instance(instance) => {
					// The analyzer chose the slot name as `<module>.<trait>@<head>`.
					if let Some((m, n)) = instance.instance_slot_name.rsplit_once('.') {
						g.reserve(m, n);
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use std::sync::atomic::{AtomicU32, Ordering};

	/// Compile a single-module source string in-process and return the
	/// checked compiler. Writes a temp `main.pa` because `from_entry_path` is
	/// the only constructor; the prelude/stdlib are embedded, so no cwd setup
	/// is needed. The temp dir is unique per call (process id + a counter) so
	/// tests running in parallel don't clobber each other's `main.pa`.
	fn check_source(src: &str) -> Compiler {
		static COUNTER: AtomicU32 = AtomicU32::new(0);
		let n = COUNTER.fetch_add(1, Ordering::Relaxed);
		let dir = std::env::temp_dir().join(format!("ir-lower-test-{}-{}", std::process::id(), n));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("main.pa");
		std::fs::File::create(&path)
			.unwrap()
			.write_all(src.as_bytes())
			.unwrap();
		let mut compiler =
			Compiler::from_entry_path(path.to_str().unwrap().to_string()).expect("from_entry_path");
		compiler.check().expect("check");
		compiler
	}

	#[test]
	fn collects_user_enum_variants() {
		let compiler = check_source("enum color {\n\tred\n\tgreen\n\tblue\n}\n\ndef n = 5\n");
		let enums = collect_enums(&compiler);
		let (_, variants) = enums
			.iter()
			.find(|(k, _)| k.ends_with(".color"))
			.expect("color enum should be collected");
		assert_eq!(
			variants,
			&vec![
				("red".to_string(), 0),
				("green".to_string(), 0),
				("blue".to_string(), 0),
			]
		);
		// Prelude enums come along for free.
		assert!(enums.keys().any(|k| k.ends_with(".option")));
	}

	#[test]
	fn reserves_prelude_and_user_def_globals() {
		let compiler = check_source("enum color {\n\tred\n\tgreen\n}\n\ndef n = 5\n");

		let mut g = GlobalTable::new();
		seed_prelude_globals(&mut g);
		let prelude_count = g.slots.len();
		assert!(g.lookup("__prelude__", "print").is_some());
		assert!(g.lookup("__prelude__", "numeric@int").is_some());

		reserve_user_globals(&mut g, &compiler);
		// The user `def n` got a slot; the `color` enum did not (enums are
		// types, not values).
		assert!(g.slots.len() > prelude_count);
		assert!(g.lookup.keys().any(|(_, name)| name == "n"));
		assert!(!g.lookup.keys().any(|(_, name)| name == "color"));
	}

	#[test]
	fn global_table_dedups_assigns_ids_and_assembles() {
		let mut g = GlobalTable::new();
		let p = g.add_pre_evaluated("m", "print", PreEval::Builtin("print".into()));
		let foo = g.reserve("m", "foo");
		assert_eq!(p, GlobalId(0));
		assert_eq!(foo, GlobalId(1));
		assert_eq!(g.lookup("m", "foo"), Some(GlobalId(1)));
		// Re-reserving the same name returns the existing slot.
		assert_eq!(g.reserve("m", "foo"), foo);

		g.set_thunk(foo, FuncId(7));
		let globals = g.finish();
		assert_eq!(globals.len(), 2);
		assert!(matches!(
			globals[0],
			GlobalInit::PreEvaluated(PreEval::Builtin(_))
		));
		assert!(matches!(globals[1], GlobalInit::Thunk(FuncId(7))));
	}
}
