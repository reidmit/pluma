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
// control flow (`if`/`when`/`while` via a pattern `Match`); data construction
// (variants + constructors, tuples, records, lists with spread, string
// interpolation, field access); and namespace access (`module.value`,
// `module.Enum.variant`) — which makes most stdlib calls work. Forms not yet
// handled (regex, trait instances, constrained-value references, nested /
// tuple / record / list patterns, `defer`, async, ...) cause the *enclosing
// def* to be lowered as a poison thunk (returns `nothing`) rather than failing
// the whole program: a def whose executed paths only touch supported forms runs
// correctly, so coverage grows fixture-by-fixture. `lower` is not yet wired into
// `codegen` as the default.

use crate::types::*;
use compiler::ast::{
	DefinitionKind, ExprKind, ExprNode, FunNode, IfNode, LetNode, LiteralKind, ModuleNode, Operator,
	PatternKind, PatternNode, WhenNode, WhileNode,
};
use compiler::types::Type;
use compiler::Compiler;
use std::collections::HashMap;

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

		let entry = self.build_entry()?;
		let test_suites: Vec<(String, GlobalId)> = self
			.compiler
			.entry_modules
			.iter()
			.filter_map(|m| self.globals.lookup(m, "tests").map(|g| (m.clone(), g)))
			.collect();

		let functions = self.functions;
		let enums = self.enums;
		let globals = self.globals.finish();
		Ok(IrProgram {
			functions,
			globals,
			enums,
			entry,
			test_suites,
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
				DefinitionKind::Instance(inst) => {
					// Trait instance dictionaries — supported later.
					if let Some((m, n)) = inst.instance_slot_name.rsplit_once('.') {
						if let Some(g) = self.globals.lookup(m, n) {
							self.poison_global(g);
						}
					}
				}
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
		// Trait-constrained defs (hidden dict params) — supported later.
		if dict_param_count > 0 {
			self.poison_global(gid);
			return;
		}
		match self.lower_thunk(name, expr) {
			Ok(fid) => self.globals.set_thunk(gid, fid),
			Err(_) => self.poison_global(gid),
		}
	}

	/// A def's value thunk: a zero-arg function that evaluates `expr` and
	/// returns it.
	fn lower_thunk(&mut self, name: &str, expr: &ExprNode) -> Result<FuncId, String> {
		let fn_name = format!("{}.{}@thunk", self.current_module, name);
		self.push_scope(fn_name, &[]);
		let atom = match self.lower_expr(expr) {
			Ok(a) => a,
			Err(e) => {
				self.scopes.pop();
				return Err(e);
			}
		};
		self.cur().stmts.push(Stmt::Return(atom));
		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	// ---- expressions ---------------------------------------------------

	fn lower_expr(&mut self, expr: &ExprNode) -> Result<Atom, String> {
		match &expr.kind {
			ExprKind::Literal(lit) => Ok(Atom::Const(literal_to_const(&lit.kind)?)),
			ExprKind::Grouping(inner) => self.lower_expr(inner),
			ExprKind::EmptyTuple => Ok(Atom::Const(Const::Unit)),
			ExprKind::Identifier(id) => {
				// A bare trait-method reference (`hash`) carries a dispatch cell.
				if let Some(cell) = &expr.trait_dispatch {
					return self.lower_dispatch(cell);
				}
				if has_undrained_dispatch(expr) {
					return Err("constrained value reference not yet supported".to_string());
				}
				self.lower_identifier(&id.name)
			}
			ExprKind::Call(call) => self.lower_call(call),
			ExprKind::Tuple(elems) => {
				let mut atoms = Vec::with_capacity(elems.len());
				for e in elems {
					atoms.push(self.lower_expr(e)?);
				}
				Ok(self.emit_let(Rvalue::MakeTuple(atoms)))
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
				Ok(self.emit_let(Rvalue::MakeList(ir_items)))
			}
			ExprKind::Record(fields) => {
				let mut ir_fields = Vec::with_capacity(fields.len());
				for (name, value) in fields {
					let atom = self.lower_expr(value)?;
					ir_fields.push((name.name.clone(), atom));
				}
				Ok(self.emit_let(Rvalue::MakeRecord(ir_fields)))
			}
			ExprKind::Interpolation(parts) => {
				let mut atoms = Vec::with_capacity(parts.len());
				for p in parts {
					atoms.push(self.lower_expr(p)?);
				}
				Ok(self.emit_let(Rvalue::Interpolate(atoms)))
			}
			ExprKind::FieldAccess { receiver, field } => {
				// Record field access (namespace/variant shapes are
				// `NamespaceAccess` by this point). If `receiver` is actually a
				// namespace it won't lower as a value, poisoning the def.
				let recv = self.lower_expr(receiver)?;
				Ok(self.emit_let(Rvalue::GetField(recv, field.name.clone())))
			}
			ExprKind::NamespaceAccess(path) => {
				if let Some(cell) = &expr.trait_dispatch {
					return self.lower_dispatch(cell);
				}
				if has_undrained_dispatch(expr) {
					return Err("constrained value reference not yet supported".to_string());
				}
				self.lower_namespace(path)
			}
			ExprKind::Fun(fun) => self.lower_fun(fun),
			ExprKind::BinaryOperation { op, left, right } => {
				self.lower_binary(expr.trait_dispatch.as_ref(), &op.kind, left, right)
			}
			ExprKind::UnaryOperation { op, right } => {
				self.lower_unary(expr.trait_dispatch.as_ref(), op, right)
			}
			ExprKind::If(n) => self.lower_if(n),
			ExprKind::When(n) => self.lower_when(n),
			ExprKind::While(n) => self.lower_while(n),
			other => Err(format!("unsupported expr: {}", expr_kind_name(other))),
		}
	}

	fn lower_identifier(&mut self, name: &str) -> Result<Atom, String> {
		match self.resolve(name)? {
			Resolved::Atom(a) => Ok(a),
			Resolved::Global(g) => Ok(self.emit_let(Rvalue::GlobalRef(g))),
			Resolved::BareVariant {
				qualified,
				variant,
				arity,
			} => self.make_variant_ref(&qualified, &variant, arity),
		}
	}

	/// A bare reference to a variant: a finished value for a nullary variant,
	/// or a constructor value for one with payload.
	fn make_variant_ref(
		&mut self,
		enum_name: &str,
		variant: &str,
		arity: usize,
	) -> Result<Atom, String> {
		if arity == 0 {
			self.make_variant(enum_name, variant, Vec::new())
		} else {
			let tag = self
				.variant_tag(enum_name, variant)
				.ok_or_else(|| format!("unknown variant `{}` of `{}`", variant, enum_name))?;
			Ok(self.emit_let(Rvalue::MakeVariantCtor {
				enum_name: enum_name.to_string(),
				tag,
			}))
		}
	}

	/// Construct an enum variant (looking up its tag in the enum table).
	fn make_variant(
		&mut self,
		enum_name: &str,
		variant: &str,
		payload: Vec<Atom>,
	) -> Result<Atom, String> {
		let tag = self
			.variant_tag(enum_name, variant)
			.ok_or_else(|| format!("unknown variant `{}` of `{}`", variant, enum_name))?;
		Ok(self.emit_let(Rvalue::MakeVariant {
			enum_name: enum_name.to_string(),
			tag,
			payload,
		}))
	}

	/// Lower a `NamespaceAccess` path: `module.value` (-> a global),
	/// `module.Enum.variant` / `Enum.variant` (-> variant construction), or a
	/// compiler-inserted fully-qualified `mod.name` reference.
	fn lower_namespace(&mut self, path: &[compiler::ast::IdentifierNode]) -> Result<Atom, String> {
		match path {
			[module, enum_name, variant] => {
				let qualified_module = self
					.imports
					.get(&module.name)
					.ok_or_else(|| format!("`{}` is not an imported module", module.name))?
					.clone();
				let qualified_enum = format!("{}.{}", qualified_module, enum_name.name);
				let arity = self.variant_arity(&qualified_enum, &variant.name)?;
				self.make_variant_ref(&qualified_enum, &variant.name, arity)
			}
			[head, tail] => {
				// A dotted head is an already-fully-qualified reference (e.g.
				// `core.task.or-else`), resolved directly against globals.
				if head.name.contains('.') {
					if let Some(g) = self.globals.lookup(&head.name, &tail.name) {
						return Ok(self.emit_let(Rvalue::GlobalRef(g)));
					}
					return Err(format!("`{}.{}` not found", head.name, tail.name));
				}
				// `module.value`.
				if let Some(qualified_module) = self.imports.get(&head.name).cloned() {
					if let Some(g) = self.globals.lookup(&qualified_module, &tail.name) {
						return Ok(self.emit_let(Rvalue::GlobalRef(g)));
					}
				}
				// `Enum.variant` for a local-module enum.
				let qualified_enum = format!("{}.{}", self.current_module, head.name);
				if self.enums.contains_key(&qualified_enum) {
					let arity = self.variant_arity(&qualified_enum, &tail.name)?;
					return self.make_variant_ref(&qualified_enum, &tail.name, arity);
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

	fn lower_call(&mut self, call: &compiler::ast::CallNode) -> Result<Atom, String> {
		if !call.dict_args.is_empty() {
			return Err("trait-constrained call (dict args) not yet supported".to_string());
		}
		let callee = self.lower_expr(&call.callee)?;
		let mut args = Vec::with_capacity(call.args.len());
		for a in &call.args {
			args.push(self.lower_expr(a)?);
		}
		Ok(self.emit_let(Rvalue::CallClosure(callee, args)))
	}

	fn lower_binary(
		&mut self,
		cell: Option<&compiler::ast::DispatchCell>,
		op: &Operator,
		left: &ExprNode,
		right: &ExprNode,
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
					let method = self.lower_dispatch(cell)?;
					let l = self.lower_expr(left)?;
					let r = self.lower_expr(right)?;
					return Ok(self.emit_let(Rvalue::CallClosure(method, vec![l, r])));
				}
				Operator::LessThan
				| Operator::LessThanEquals
				| Operator::GreaterThan
				| Operator::GreaterThanEquals => {
					// `ord.compare(left, right)` then test the resulting ordering
					// variant: `< == lt`, `> == gt`, `<= != gt`, `>= != lt`.
					let method = self.lower_dispatch(cell)?;
					let l = self.lower_expr(left)?;
					let r = self.lower_expr(right)?;
					let cmp = self.emit_let(Rvalue::CallClosure(method, vec![l, r]));
					let (variant, use_ne) = match op {
						Operator::LessThan => ("lt", false),
						Operator::GreaterThan => ("gt", false),
						Operator::LessThanEquals => ("gt", true),
						Operator::GreaterThanEquals => ("lt", true),
						_ => unreachable!(),
					};
					let v = self.make_variant("__prelude__.ordering", variant, Vec::new())?;
					let binop = if use_ne { BinOp::Ne } else { BinOp::Eq };
					return Ok(self.emit_let(Rvalue::Bin(binop, cmp, v)));
				}
				_ => return Err("unsupported dispatched operator".to_string()),
			}
		}
		// `x | f a b` pipes `x` in as `f`'s first argument.
		if let Operator::Chain = op {
			return self.lower_chain(left, right);
		}
		// Concrete, non-dispatched operator: a direct VM opcode picked by
		// operand type. Evaluate left then right (matching `emit.rs`).
		let is_float = matches!(left.ty, Type::Float) || matches!(right.ty, Type::Float);
		let binop = binop_for(op, is_float).ok_or("unsupported binary operator")?;
		let l = self.lower_expr(left)?;
		let r = self.lower_expr(right)?;
		Ok(self.emit_let(Rvalue::Bin(binop, l, r)))
	}

	fn lower_unary(
		&mut self,
		cell: Option<&compiler::ast::DispatchCell>,
		op: &Operator,
		right: &ExprNode,
	) -> Result<Atom, String> {
		// Numeric `-` (negate) is trait-dispatched (`numeric.negate`); the only
		// direct unary op is logical `!`.
		if let Some(cell) = cell {
			let method = self.lower_dispatch(cell)?;
			let r = self.lower_expr(right)?;
			return Ok(self.emit_let(Rvalue::CallClosure(method, vec![r])));
		}
		match op {
			Operator::LogicalNot => {
				let r = self.lower_expr(right)?;
				Ok(self.emit_let(Rvalue::Not(r)))
			}
			_ => Err("unsupported unary operator".to_string()),
		}
	}

	/// Lower a resolved trait-method dispatch cell to the callable method
	/// value. Handles concrete instances (`Resolved::Global`): load the named
	/// dictionary global and extract the method. `Forwarded` (dict params of a
	/// constrained def) and `InstanceChain` (parametric instances) aren't
	/// ported yet.
	fn lower_dispatch(&mut self, cell: &compiler::ast::DispatchCell) -> Result<Atom, String> {
		use compiler::ast::Resolved;
		let borrow = cell.borrow();
		let method_idx = borrow.method_idx;
		let gid = match &borrow.resolved {
			Some(Resolved::Global(slot_name)) => {
				let (module, name) = slot_name
					.rsplit_once('.')
					.ok_or("malformed instance slot name")?;
				self
					.globals
					.lookup(module, name)
					.ok_or("instance slot not registered as a global")?
			}
			Some(Resolved::Forwarded(_)) => {
				return Err("forwarded dispatch (constrained def) not yet supported".to_string())
			}
			Some(Resolved::InstanceChain { .. }) => {
				return Err("parametric instance dispatch not yet supported".to_string())
			}
			None => return Err("unresolved dispatch cell".to_string()),
		};
		drop(borrow);
		let dict = self.emit_let(Rvalue::GlobalRef(gid));
		match method_idx {
			Some(idx) => Ok(self.emit_let(Rvalue::GetDictMethod(dict, idx as u32))),
			// A call-forwarding site pushes the whole dict; operators always
			// name a method, so this branch is unused for them.
			None => Ok(dict),
		}
	}

	/// `left | right` — pipe `left` in as the first argument of the call on the
	/// right (or, if `right` isn't a call, call it with the single argument).
	fn lower_chain(&mut self, left: &ExprNode, right: &ExprNode) -> Result<Atom, String> {
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
		Ok(self.emit_let(Rvalue::CallClosure(callee_atom, args)))
	}

	// ---- control flow ---------------------------------------------------

	/// `if subject is pattern { body } [else { else_body }]`. Lowers to a
	/// `Match`: the pattern arm, plus a wildcard `else` arm when present. The
	/// result lives in a fresh var (defaulting to `nothing` — which is exactly
	/// the value of an `else`-less `if`, since its body value is discarded).
	fn lower_if(&mut self, n: &IfNode) -> Result<Atom, String> {
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
		self.cur().stmts.push(Stmt::Match { subject, arms });
		Ok(Atom::Var(result))
	}

	/// `when subject is p1 { b1 } is p2 { b2 } ...`. Each case is a match arm;
	/// the arm bodies all write the shared result var.
	fn lower_when(&mut self, n: &WhenNode) -> Result<Atom, String> {
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
		self.cur().stmts.push(Stmt::Match { subject, arms });
		Ok(Atom::Var(result))
	}

	/// `while subject is pattern { body }`. A `Loop` that re-evaluates the
	/// subject each iteration and matches it: on match, run the body and
	/// continue; otherwise break. Evaluates to `nothing`.
	fn lower_while(&mut self, n: &WhileNode) -> Result<Atom, String> {
		let saved = self.take_stmts();
		let res = self.lower_while_body(n);
		let loop_stmts = self.restore_stmts(saved);
		res?;
		self.cur().stmts.push(Stmt::Loop(Block(loop_stmts)));
		Ok(Atom::Const(Const::Unit))
	}

	fn lower_while_body(&mut self, n: &WhileNode) -> Result<(), String> {
		let subject = self.lower_expr(&n.subject)?;
		let mark = self.cur().locals.len();
		let pattern = self.lower_pattern(&n.pattern, &n.subject.ty)?;
		let mut matched = self.lower_block_of(&n.body, None)?;
		matched.0.push(Stmt::Continue);
		self.cur().locals.truncate(mark);
		let arms = vec![
			MatchArm {
				pattern,
				body: matched,
			},
			MatchArm {
				pattern: Pattern::Wildcard,
				body: Block(vec![Stmt::Break]),
			},
		];
		self.cur().stmts.push(Stmt::Match { subject, arms });
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
		let assign = |s: &mut Self, atom: Atom| {
			if let Some(r) = result {
				s.cur().stmts.push(Stmt::Let(r, Rvalue::Use(atom)));
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
				let mut fields = Vec::with_capacity(subs.len());
				for sub in subs {
					fields.push(self.lower_field_pattern(sub)?);
				}
				Ok(Pattern::Variant {
					variant: variant.name.clone(),
					fields,
				})
			}
			_ => Err("tuple / record / list pattern not yet supported".to_string()),
		}
	}

	/// A variant payload sub-pattern. Only simple binds and `_` are supported;
	/// a nested pattern (or an identifier that names a nullary variant, which
	/// would be a nested match rather than a binding) poisons the def.
	fn lower_field_pattern(&mut self, sub: &PatternNode) -> Result<FieldPat, String> {
		match &sub.kind {
			PatternKind::Underscore => Ok(FieldPat::Wildcard),
			PatternKind::Identifier(id) => {
				let names_variant = self
					.enums
					.values()
					.any(|vs| vs.iter().any(|(n, a)| n == &id.name && *a == 0));
				if names_variant {
					return Err("nested nullary-variant sub-pattern not yet supported".to_string());
				}
				let v = self.alloc_var();
				self.cur().locals.push((id.name.clone(), v));
				Ok(FieldPat::Bind(v))
			}
			_ => Err("nested sub-pattern not yet supported".to_string()),
		}
	}

	fn lower_fun(&mut self, fun: &FunNode) -> Result<Atom, String> {
		let param_names: Vec<&str> = fun.params.iter().map(|p| p.ident.name.as_str()).collect();
		let fn_name = format!(
			"{}.fun@{}:{}",
			self.current_module, fun.range.start.line, fun.range.start.col
		);
		self.push_scope(fn_name, &param_names);
		let atom = match self.lower_body(&fun.body) {
			Ok(a) => a,
			Err(e) => {
				self.scopes.pop();
				return Err(e);
			}
		};
		self.cur().stmts.push(Stmt::Return(atom));
		let scope = self.scopes.pop().unwrap();

		// Build the closure's capture list (resolved against the now-current
		// parent scope) before consuming `scope`.
		let capture_atoms: Vec<Atom> = scope
			.captures
			.iter()
			.map(|c| self.capture_src_atom(&c.src))
			.collect();
		let fid = self.add_function(finish_scope(scope));
		Ok(self.emit_let(Rvalue::MakeClosure(fid, capture_atoms)))
	}

	/// Lower a function/`let`-block body (a sequence of statements). Returns
	/// the value the body evaluates to (its last expression).
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

	fn lower_let(&mut self, let_node: &LetNode) -> Result<(), String> {
		match &let_node.pattern.kind {
			PatternKind::Identifier(id) => {
				let atom = self.lower_expr(&let_node.value)?;
				let var = match atom {
					Atom::Var(v) => v,
					other => {
						let v = self.alloc_var();
						self.cur().stmts.push(Stmt::Let(v, Rvalue::Use(other)));
						v
					}
				};
				self.cur().locals.push((id.name.clone(), var));
				Ok(())
			}
			_ => Err("refutable / destructuring `let` not yet supported".to_string()),
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

	fn emit_let(&mut self, rv: Rvalue) -> Atom {
		let v = self.alloc_var();
		self.cur().stmts.push(Stmt::Let(v, rv));
		Atom::Var(v)
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

	fn build_entry(&mut self) -> Result<FuncId, String> {
		let main_module = self
			.compiler
			.entry_modules
			.first()
			.ok_or("no entry module")?
			.clone();
		let main = self
			.globals
			.lookup(&main_module, "main")
			.ok_or_else(|| format!("module `{}` has no `main` def", main_module))?;
		// Load `main`, call it with the unit arg, return the result.
		let func = Function {
			name: "__entry__".to_string(),
			module: String::new(),
			params: Vec::new(),
			captures: Vec::new(),
			is_async: false,
			body: Block(vec![
				Stmt::Let(VarId(0), Rvalue::GlobalRef(main)),
				Stmt::Let(
					VarId(1),
					Rvalue::CallClosure(Atom::Var(VarId(0)), vec![Atom::Const(Const::Unit)]),
				),
				Stmt::Return(Atom::Var(VarId(1))),
			]),
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
			body: Block(vec![Stmt::Return(Atom::Const(Const::Unit))]),
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

/// True when an expression carries an undrained dispatch sink — a
/// trait-constrained value referenced in value position (not directly called),
/// which needs its dictionaries pre-applied. Not ported yet, so such sites
/// poison their def. Mirrors `codegen::emit::undrained_dispatch_cells`.
fn has_undrained_dispatch(expr: &ExprNode) -> bool {
	expr
		.dispatch_sink
		.as_ref()
		.map_or(false, |sink| !sink.borrow().is_empty())
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
		(Operator::LessThan, _) => BinOp::Lt,
		(Operator::LessThanEquals, _) => BinOp::Le,
		(Operator::GreaterThan, _) => BinOp::Gt,
		(Operator::GreaterThanEquals, _) => BinOp::Ge,
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
		LiteralKind::Duration(_) => return Err("duration literal not yet supported".to_string()),
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
