// Lowering: typed AST -> IR.
//
// `ir::lower(&compiler)` is the sole front-to-IR path; the WASM backend
// (`wasm::emit`) consumes its output. This is where every backend-independent
// elaboration lives:
//   * identifier resolution (locals / captures / globals)
//   * closure conversion (explicit capture lists)
//   * dictionary elaboration (trait constraints -> dict params + GetDictMethod)
//   * pattern compilation (`when`/`if is` -> Switch + GetTag/GetPayload)
//   * `defer` edge insertion
//   * async marking (`Function::is_async` + `Await`)
//   * two standalone pre-passes (enum table + global reservation)
//
// It covers the full language surface: literals, identifiers (local / capture /
// global), calls, `fun` (closure conversion), `let` (incl. irrefutable
// destructuring); operators (direct ops + trait dispatch via method
// dictionaries); control flow (`if`/`when`/`while` via a pattern `Match`, with
// literal / variant / tuple / record / list patterns, nested and with `...`
// rests); data construction (variants + constructors, tuples, records, lists with
// spread, string interpolation, field access, regex literals); namespace access
// (`module.value`, `module.Enum.variant`); the full trait-dictionary machinery
// (instance defs, constrained defs, every dispatch shape, constrained calls and
// value references); `defer`; async/`Await`; and duration literals. A def that
// hits a genuinely-unsupported form is lowered as a *poison thunk* (returns
// `nothing`) rather than failing the whole program.

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

/// Lower for `pluma test`: synthesize an entry that runs every module's `tests`
/// suite through `std.test.run-all`, rather than calling a `main`. `color`
/// enables ANSI styling in the rendered report.
pub fn lower_tests(compiler: &Compiler, color: bool) -> Result<IrProgram, String> {
	let mut lowerer = Lowerer::new(compiler);
	lowerer.test_color = Some(color);
	lowerer.run()
}

/// Lower a FULLSTACK build rooted at a specific entry module's `main` (`server` or
/// `client`), overriding `entry_modules[0]`. The whole analyzed program is lowered;
/// only the program *entry* differs, so the emitter's reachability prune yields the
/// one artifact's functions (the other side's code is never reached).
pub fn lower_entry(compiler: &Compiler, entry: &str) -> Result<IrProgram, String> {
	let mut lowerer = Lowerer::new(compiler);
	lowerer.entry_override = Some(entry.to_string());
	lowerer.run()
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
	// `Some(color)` when lowering for `pluma test`: `build_entry` then synthesizes
	// a runner over every `tests` suite instead of the module's `main`.
	test_color: Option<bool>,
	// Which module's `main` is the program entry, overriding `entry_modules[0]`.
	// Set for a FULLSTACK dual build, which lowers the one analyzed program twice —
	// once rooted at `server`'s `main`, once at `client`'s — and lets the emitter's
	// reachability prune carve out each artifact (the server-only `remote def` bodies
	// are simply never reached from the client's `main`).
	entry_override: Option<String>,
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
	// Repr of each param (parallel to `params`) and of the body's tail value.
	// All-`Boxed` by default; `lower_closure` overwrites them from the AST types
	// for `fun` bodies so the step-2 monomorphization pass can read each concrete
	// function's signature. Carried into `Function` by `finish_scope`.
	param_reprs: Vec<Repr>,
	ret_repr: Repr,
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
		// Native modules currently contribute no globals (none are registered —
		// every stdlib module is `.pa` source). When a Rust-defined native module
		// is registered, its defs/constants would be seeded here as `PreEval`.
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
			test_color: None,
			entry_override: None,
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
		// In test mode (`lower_tests`), synthesize a runner over these suites;
		// otherwise the entry is the module's `main` (with a no-op fallback for a
		// suite-bearing module that has no `main`).
		let entry = self.build_entry(&test_suites)?;

		// Annotate every function's bindings with a `Repr` (uniform-boxed except
		// arithmetic/comparison/`Not` results and primitive literals). The WASM
		// backend maps each repr to a native/GC-ref local.
		let mut functions = self.functions;
		for f in &mut functions {
			f.var_reprs = crate::repr::infer_reprs(f, &crate::repr::Sigs::uniform());
		}
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
				DefinitionKind::Expr(expr) => self.lower_value_def(
					module,
					&def.name.name,
					def.dict_param_count,
					def.is_remote,
					expr,
				),
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

	fn lower_value_def(
		&mut self,
		module: &str,
		name: &str,
		dict_param_count: u16,
		is_remote: bool,
		expr: &ExprNode,
	) {
		let gid = match self.globals.lookup(module, name) {
			Some(g) => g,
			None => return,
		};
		// On the client artifact, a `remote def`'s written body is dead — replace it
		// with a transport stub (encode args → `rpc.call-unary` → decode reply). The
		// emitter's DCE then drops the server-only helpers the real body referenced.
		// (A remote def whose contract failed validation has no metadata; fall through
		// and lower its real body so the analyzer's error is the one surfaced.)
		if is_remote && self.is_client_emit() {
			if let Some(ep) = self.rpc_endpoint(module, name) {
				match self
					.synthesize_client_stub(name, &ep)
					.and_then(|stub| self.thunk_returning_closure("rpc-stub", stub))
				{
					Ok(thunk) => self.globals.set_thunk(gid, thunk),
					Err(_) => self.poison_global(gid),
				}
				return;
			}
		}
		// `built-in "tag"` RHS: a pre-evaluated builtin value, no thunk. Capture the
		// builtin's *declared* return repr from its annotated type (`def get :: fun
		// bytes int -> int`) so a deploy backend can read a scalar-returning builtin's
		// result unboxed — the analyzer already knows this type, so don't discard it.
		if let ExprKind::Builtin(tag) = &expr.kind {
			// The RPC builtins aren't host imports: their bodies are synthesized from
			// the discovered endpoints (`rpc-dispatch`) or baked from a build flag
			// (`rpc-server-origin`). Intercept before the generic host-builtin path.
			if tag == "rpc-dispatch" {
				match self
					.synthesize_dispatch()
					.and_then(|f| self.thunk_returning_closure("dispatch", f))
				{
					Ok(thunk) => self.globals.set_thunk(gid, thunk),
					Err(_) => self.poison_global(gid),
				}
				return;
			}
			if tag == "rpc-server-origin" {
				match self.synthesize_server_origin() {
					Ok(fid) => self.globals.set_thunk(gid, fid),
					Err(_) => self.poison_global(gid),
				}
				return;
			}
			let ret = match &expr.ty {
				Type::Fun(_, ret) => crate::repr::repr_of_type(ret),
				_ => Repr::Boxed,
			};
			self
				.globals
				.set_pre_evaluated(gid, PreEval::Builtin(tag.clone(), ret));
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

	/// Wrap a synthesized parameter-taking function in a def's value thunk: a
	/// zero-arg function returning `MakeClosure(inner, [])`. A top-level def's
	/// global is the *thunk*, evaluated lazily to the closure value — so a
	/// synthesized `fun … { … }` (dispatch, a client stub) must be wrapped this
	/// way, exactly as `lower_thunk` does for a written `def f = fun …`.
	fn thunk_returning_closure(&mut self, label: &str, inner: FuncId) -> Result<FuncId, String> {
		self.push_scope(format!("{}.{}@thunk", self.current_module, label), &[]);
		let c = self.emit_let(Rvalue::MakeClosure(inner, Vec::new()), SYNTHETIC);
		self.push_stmt(StmtKind::Return(c), SYNTHETIC);
		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	// ---- RPC synthesis (client stubs + server dispatch) ----------------

	/// Whether this emit is the *client* artifact: every `remote def` body is
	/// swapped for a transport stub. A fullstack build picks the side by entry
	/// (`entry_modules` is `[server, client]`); a single build keys on the Web
	/// target; `pluma run`/`test` are server context (real handler bodies, so an
	/// in-process endpoint test calls the handler directly — RPC.md §7a).
	fn is_client_emit(&self) -> bool {
		if self.compiler.fullstack {
			matches!(
				(self.entry_override.as_deref(), self.compiler.entry_modules.get(1)),
				(Some(e), Some(client)) if e == client.as_str()
			)
		} else {
			self.compiler.target == Some(compiler::Target::Web)
		}
	}

	/// The validated metadata for `<module>.<name>` if it's a known endpoint.
	/// Cloned so the borrow of `self.compiler` doesn't outlive the `&mut self`
	/// synthesis calls.
	fn rpc_endpoint(&self, module: &str, name: &str) -> Option<compiler::rpc::RpcEndpointMeta> {
		self
			.compiler
			.rpc_endpoints
			.iter()
			.find(|e| e.module == module && e.name == name)
			.cloned()
	}

	/// Load a top-level global as an atom, for synthesized IR that references the
	/// stdlib / prelude / handler globals by qualified name.
	fn rpc_global(&mut self, module: &str, name: &str, range: Range) -> Result<Atom, String> {
		let g = self
			.globals
			.lookup(module, name)
			.ok_or_else(|| format!("`{}.{}` not registered (RPC synthesis)", module, name))?;
		Ok(self.emit_let(Rvalue::GlobalRef(g), range))
	}

	/// `task.map task fn` / `task.then task fn` — a call through the std.task
	/// global, the spine of every synthesized stub/dispatch.
	fn task_combinator(&mut self, op: &str, task: Atom, f: Atom) -> Result<Atom, String> {
		let g = self.rpc_global("std.task", op, SYNTHETIC)?;
		Ok(self.emit_let(Rvalue::CallClosure(g, vec![task, f]), SYNTHETIC))
	}

	/// `task.return value` as an atom.
	fn task_return(&mut self, value: Atom) -> Result<Atom, String> {
		let g = self.rpc_global("std.task", "return", SYNTHETIC)?;
		Ok(self.emit_let(Rvalue::CallClosure(g, vec![value]), SYNTHETIC))
	}

	/// The client stub for one endpoint:
	/// `fun a0..an { task.then (rpc.call-unary "<route>" "<fp>" (wire.encode <argschema> <args>))
	///               (fun b { rpc.lift (wire.decode <resultschema> b) }) }`.
	/// Returns the stub function's id; the def's global thunk returns its closure.
	fn synthesize_client_stub(
		&mut self,
		name: &str,
		ep: &compiler::rpc::RpcEndpointMeta,
	) -> Result<FuncId, String> {
		// The stub function `fun a0..an { ... }`.
		let param_names: Vec<String> = (0..ep.arity).map(|i| format!("a{}", i)).collect();
		let param_refs: Vec<&str> = param_names.iter().map(String::as_str).collect();
		self.push_scope(
			format!("{}.{}@rpc-stub", self.current_module, name),
			&param_refs,
		);
		let params: Vec<VarId> = self.scopes.last().unwrap().params.clone();

		// Encode the arguments to bytes: nothing for arity 0, the single value for
		// arity 1, an encoded tuple for arity ≥ 2 (matching the dispatch decode).
		let body_bytes = if ep.arity == 0 {
			Atom::Const(Const::Bytes(Vec::new()))
		} else {
			let value = if ep.arity == 1 {
				Atom::Var(params[0])
			} else {
				let elems: Vec<Atom> = params.iter().map(|v| Atom::Var(*v)).collect();
				self.emit_let(Rvalue::MakeTuple(elems), SYNTHETIC)
			};
			let schema = self.lower_wire_shape(&ep.arg_shape, SYNTHETIC)?;
			let enc = self.rpc_global("__prelude__", "wire-encode", SYNTHETIC)?;
			self.emit_let(Rvalue::CallClosure(enc, vec![schema, value]), SYNTHETIC)
		};

		// The decode callback `fun b { rpc.lift (wire.decode <rs> b) }` (`fun bytes
		// -> task R`) is shared by both kinds — unary threads it through
		// `task.then`, a stream maps it over every frame with `stream.map-task` (a
		// decode failure faults that arm the same way it fails a unary call).
		let route = Atom::Const(Const::Str(ep.route()));
		let fp = Atom::Const(Const::Str(ep.route_fp.clone()));
		let decode_cb = self.synthesize_decode_closure(&ep.result_shape)?;
		let result = match ep.kind {
			compiler::rpc::EndpointKind::Unary => {
				// rpc.call-unary "<route>" "<fp>" body  →  task bytes
				let call_unary = self.rpc_global("std.rpc", "call-unary", SYNTHETIC)?;
				let task_bytes = self.emit_let(
					Rvalue::CallClosure(call_unary, vec![route, fp, body_bytes]),
					SYNTHETIC,
				);
				self.task_combinator("then", task_bytes, decode_cb)?
			}
			compiler::rpc::EndpointKind::Stream => {
				// rpc.call-stream "<route>" "<fp>" body  →  stream bytes
				let call_stream = self.rpc_global("std.rpc", "call-stream", SYNTHETIC)?;
				let frames = self.emit_let(
					Rvalue::CallClosure(call_stream, vec![route, fp, body_bytes]),
					SYNTHETIC,
				);
				// stream.map-task frames decode-cb  →  stream R
				let map_task = self.rpc_global("std.stream", "map-task", SYNTHETIC)?;
				self.emit_let(
					Rvalue::CallClosure(map_task, vec![frames, decode_cb]),
					SYNTHETIC,
				)
			}
		};
		self.push_stmt(StmtKind::Return(result), SYNTHETIC);

		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	/// The stub's decode callback: `fun b { rpc.lift (wire.decode <resultschema> b) }`.
	/// The result schema is lowered *inside* this closure's scope so its helper
	/// `let`s land here, not in the caller. Captures nothing; returned as a
	/// `MakeClosure` atom emitted into the caller's block.
	fn synthesize_decode_closure(
		&mut self,
		result_shape: &compiler::ast::WireShape,
	) -> Result<Atom, String> {
		self.push_scope(format!("{}@rpc-decode", self.current_module), &["b"]);
		let b = Atom::Var(self.scopes.last().unwrap().params[0]);
		let schema = self.lower_wire_shape(result_shape, SYNTHETIC)?;
		let dec = self.rpc_global("__prelude__", "wire-decode", SYNTHETIC)?;
		let decoded = self.emit_let(Rvalue::CallClosure(dec, vec![schema, b]), SYNTHETIC);
		let lift = self.rpc_global("std.rpc", "lift", SYNTHETIC)?;
		let lifted = self.emit_let(Rvalue::CallClosure(lift, vec![decoded]), SYNTHETIC);
		self.push_stmt(StmtKind::Return(lifted), SYNTHETIC);
		let scope = self.scopes.pop().unwrap();
		let fid = self.add_function(finish_scope(scope));
		Ok(self.emit_let(Rvalue::MakeClosure(fid, Vec::new()), SYNTHETIC))
	}

	/// The server `dispatch` handler, synthesized from every discovered endpoint:
	/// `fun req { match req.path { "/rpc/<route>" => <decode→call→encode> … _ => 404 } }`.
	/// With no endpoints it's an always-404 router. The function isn't `is_async`
	/// (it builds task values and returns them; the server awaits the result).
	fn synthesize_dispatch(&mut self) -> Result<FuncId, String> {
		let endpoints = self.compiler.rpc_endpoints.clone();
		self.push_scope(format!("{}.dispatch@rpc", self.current_module), &["req"]);
		let req = Atom::Var(self.scopes.last().unwrap().params[0]);
		let path = self.emit_let(
			Rvalue::GetField(req.clone(), "path".to_string(), None),
			SYNTHETIC,
		);
		let result = self.alloc_var();

		let mut arms: Vec<MatchArm> = Vec::new();
		for ep in &endpoints {
			let route_path = format!("/rpc/{}", ep.route());
			let saved = self.take_stmts();
			let r = self.build_dispatch_arm(&req, result, ep);
			let block = Block(self.restore_stmts(saved));
			r?;
			arms.push(MatchArm {
				pattern: Pattern::Literal(Const::Str(route_path)),
				body: block,
			});
		}

		// Default: no such route → 404.
		let saved = self.take_stmts();
		let nf = self.rpc_global("std.sys.http", "not-found", SYNTHETIC)?;
		let resp = self.emit_let(
			Rvalue::CallClosure(nf, vec![Atom::Const(Const::Unit)]),
			SYNTHETIC,
		);
		let task_nf = self.task_return(resp)?;
		self.push_stmt(StmtKind::Let(result, Rvalue::Use(task_nf)), SYNTHETIC);
		let default_block = Block(self.restore_stmts(saved));
		arms.push(MatchArm {
			pattern: Pattern::Wildcard,
			body: default_block,
		});

		self.push_stmt(
			StmtKind::Match {
				subject: path,
				arms,
			},
			SYNTHETIC,
		);
		self.push_stmt(StmtKind::Return(Atom::Var(result)), SYNTHETIC);
		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	/// One dispatch arm: guard the per-route fingerprint, then invoke the
	/// endpoint. A mismatch (or missing fingerprint) short-circuits to a 409
	/// schema-skew response without decoding. Assigns the `task response` to
	/// `result`. Emits into the caller's (the arm's) block buffer.
	fn build_dispatch_arm(
		&mut self,
		req: &Atom,
		result: VarId,
		ep: &compiler::rpc::RpcEndpointMeta,
	) -> Result<(), String> {
		let fpok_fn = self.rpc_global("std.rpc", "fingerprint-ok", SYNTHETIC)?;
		let fpok = self.emit_let(
			Rvalue::CallClosure(
				fpok_fn,
				vec![req.clone(), Atom::Const(Const::Str(ep.route_fp.clone()))],
			),
			SYNTHETIC,
		);

		// then: fingerprint matches → decode, call, encode.
		let then_saved = self.take_stmts();
		let invoke = self.dispatch_invoke(req, result, ep);
		let then_block = Block(self.restore_stmts(then_saved));
		invoke?;

		// else: stale/missing fingerprint → 409.
		let else_saved = self.take_stmts();
		let skew = self.rpc_global("std.rpc", "skew-response", SYNTHETIC)?;
		let resp = self.emit_let(
			Rvalue::CallClosure(skew, vec![Atom::Const(Const::Unit)]),
			SYNTHETIC,
		);
		let task_skew = self.task_return(resp)?;
		self.push_stmt(StmtKind::Let(result, Rvalue::Use(task_skew)), SYNTHETIC);
		let else_block = Block(self.restore_stmts(else_saved));

		// A `Match` on the bool, mirroring how `if` lowers (a `Literal(Bool(true))`
		// arm + a wildcard else) — `lower_if` itself compiles to a `Match`, so this
		// is the path the backend's repr coercion expects for a boxed condition.
		self.push_stmt(
			StmtKind::Match {
				subject: fpok,
				arms: vec![
					MatchArm {
						pattern: Pattern::Literal(Const::Bool(true)),
						body: then_block,
					},
					MatchArm {
						pattern: Pattern::Wildcard,
						body: else_block,
					},
				],
			},
			SYNTHETIC,
		);
		Ok(())
	}

	/// Invoke one endpoint (assuming its fingerprint already checked out): decode
	/// the request body against the arg shape, then route the handler through
	/// `std.rpc.respond` (ambient context + reject + 200/4xx/5xx shaping). A
	/// malformed body short-circuits to 400 before the handler runs. Assigns the
	/// `task response` to `result`. Emits into the caller's block buffer.
	fn dispatch_invoke(
		&mut self,
		req: &Atom,
		result: VarId,
		ep: &compiler::rpc::RpcEndpointMeta,
	) -> Result<(), String> {
		if ep.arity == 0 {
			// Nothing to decode — invoke straight through `respond`.
			return self.dispatch_respond(req, result, ep, &[]);
		}

		// Decode the argument(s): `wire.decode <argschema> req.body` → `result args string`.
		let body = self.emit_let(
			Rvalue::GetField(req.clone(), "body".to_string(), None),
			SYNTHETIC,
		);
		let schema = self.lower_wire_shape(&ep.arg_shape, SYNTHETIC)?;
		let dec = self.rpc_global("__prelude__", "wire-decode", SYNTHETIC)?;
		let decoded = self.emit_let(Rvalue::CallClosure(dec, vec![schema, body]), SYNTHETIC);

		// Bind each argument: a single var for arity 1, a tuple pattern for arity ≥ 2.
		let argvars: Vec<VarId> = (0..ep.arity).map(|_| self.alloc_var()).collect();
		let argpat = if ep.arity == 1 {
			Pattern::Bind(argvars[0])
		} else {
			Pattern::Tuple(argvars.iter().map(|v| Pattern::Bind(*v)).collect())
		};

		// ok arm: invoke the handler through `respond` (binds context + reject).
		let ok_saved = self.take_stmts();
		let invoke = self.dispatch_respond(req, result, ep, &argvars);
		let ok_block = Block(self.restore_stmts(ok_saved));
		invoke?;

		// err arm: a malformed request body → 400.
		let err_saved = self.take_stmts();
		let text = self.rpc_global("std.sys.http", "text", SYNTHETIC)?;
		let resp = self.emit_let(
			Rvalue::CallClosure(
				text,
				vec![
					Atom::Const(Const::Int(400)),
					Atom::Const(Const::Str("rpc: malformed request".to_string())),
				],
			),
			SYNTHETIC,
		);
		let task_400 = self.task_return(resp)?;
		self.push_stmt(StmtKind::Let(result, Rvalue::Use(task_400)), SYNTHETIC);
		let err_block = Block(self.restore_stmts(err_saved));

		self.push_stmt(
			StmtKind::Match {
				subject: decoded,
				arms: vec![
					MatchArm {
						pattern: Pattern::Variant {
							variant: "ok".to_string(),
							tag: self.pattern_variant_tag("__prelude__.result", "ok")?,
							fields: vec![argpat],
						},
						body: ok_block,
					},
					MatchArm {
						pattern: Pattern::Variant {
							variant: "err".to_string(),
							tag: self.pattern_variant_tag("__prelude__.result", "err")?,
							fields: vec![Pattern::Wildcard],
						},
						body: err_block,
					},
				],
			},
			SYNTHETIC,
		);
		Ok(())
	}

	/// A handler's result encoder: `fun res { wire.encode <resultschema> res }`,
	/// producing the reply bytes. The 200-response wrapping lives in `std.rpc`'s
	/// `respond` (which also binds context and catches a `reject`); this closure
	/// just turns the handler's value into bytes. The schema is lowered inside the
	/// closure. Captures nothing.
	fn synthesize_encode_bytes_closure(
		&mut self,
		result_shape: &compiler::ast::WireShape,
	) -> Result<Atom, String> {
		self.push_scope(format!("{}@rpc-encode", self.current_module), &["res"]);
		let res = Atom::Var(self.scopes.last().unwrap().params[0]);
		let schema = self.lower_wire_shape(result_shape, SYNTHETIC)?;
		let enc = self.rpc_global("__prelude__", "wire-encode", SYNTHETIC)?;
		let bytes = self.emit_let(Rvalue::CallClosure(enc, vec![schema, res]), SYNTHETIC);
		self.push_stmt(StmtKind::Return(bytes), SYNTHETIC);
		let scope = self.scopes.pop().unwrap();
		let fid = self.add_function(finish_scope(scope));
		Ok(self.emit_let(Rvalue::MakeClosure(fid, Vec::new()), SYNTHETIC))
	}

	/// The thunk `std.rpc.respond` runs to invoke one endpoint:
	/// `fun { task.map (<module>.<name> <args>) (fun res { wire.encode <rs> res }) }`
	/// — a zero-arg closure capturing the decoded argument vars from the arm. The
	/// handler is invoked *inside* this closure (not eagerly), because `respond`
	/// runs it under the ambient `context` + `reject` bindings: capturing the args
	/// and deferring the call is what places the handler's invocation — and any
	/// synchronous `context.*` read it does — inside those bindings.
	fn synthesize_invoke_thunk(
		&mut self,
		ep: &compiler::rpc::RpcEndpointMeta,
		args: &[VarId],
	) -> Result<Atom, String> {
		self.push_scope(
			format!("{}.{}@rpc-invoke", self.current_module, ep.name),
			&[],
		);
		let inner = self.scopes.len() - 1;
		// Capture each decoded argument from the parent (dispatch) scope.
		let arg_atoms: Vec<Atom> = args
			.iter()
			.enumerate()
			.map(|(i, &pv)| {
				let slot = self.add_capture(inner, &format!("a{}", i), CaptureSrc::ParentLocal(pv));
				match slot {
					ScopeSlot::Capture(ci) => Atom::Var(self.scopes[inner].captures[ci].var),
					_ => unreachable!("add_capture returns a Capture slot"),
				}
			})
			.collect();
		let handler = self.rpc_global(&ep.module, &ep.name, SYNTHETIC)?;
		// Arity 0 takes the unit arg; otherwise pass the captured arguments.
		let call_args = if args.is_empty() {
			vec![Atom::Const(Const::Unit)]
		} else {
			arg_atoms
		};
		let produced = self.emit_let(Rvalue::CallClosure(handler, call_args), SYNTHETIC);
		let enc = self.synthesize_encode_bytes_closure(&ep.result_shape)?;
		// Encode the handler's result to bytes: a unary `task R` maps through
		// `task.map` (→ task bytes); a `stream R` maps through `stream.map`
		// (→ stream bytes). `respond` / `respond-stream` consume the matching shape.
		let mapped = match ep.kind {
			compiler::rpc::EndpointKind::Unary => self.task_combinator("map", produced, enc)?,
			compiler::rpc::EndpointKind::Stream => {
				let smap = self.rpc_global("std.stream", "map", SYNTHETIC)?;
				self.emit_let(Rvalue::CallClosure(smap, vec![produced, enc]), SYNTHETIC)
			}
		};
		self.push_stmt(StmtKind::Return(mapped), SYNTHETIC);
		let scope = self.scopes.pop().unwrap();
		let capture_atoms: Vec<Atom> = scope
			.captures
			.iter()
			.map(|c| self.capture_src_atom(&c.src))
			.collect();
		let fid = self.add_function(finish_scope(scope));
		Ok(self.emit_let(Rvalue::MakeClosure(fid, capture_atoms), SYNTHETIC))
	}

	/// `result = std.rpc.respond <req> <invoke-thunk>` — hand the inbound request
	/// and the endpoint's invocation thunk to `respond`, which binds the ambient
	/// context + reject box, runs the handler, and shapes success / `reject` / fault
	/// into an HTTP response. Emits into the caller's block buffer.
	fn dispatch_respond(
		&mut self,
		req: &Atom,
		result: VarId,
		ep: &compiler::rpc::RpcEndpointMeta,
		args: &[VarId],
	) -> Result<(), String> {
		let thunk = self.synthesize_invoke_thunk(ep, args)?;
		// Unary endpoints shape a `task bytes` into a 200; streams shape a `stream
		// bytes` into a streaming (SSE) 200. Both bind the request as ambient
		// `context` before running the thunk.
		let respond_name = match ep.kind {
			compiler::rpc::EndpointKind::Unary => "respond",
			compiler::rpc::EndpointKind::Stream => "respond-stream",
		};
		let respond = self.rpc_global("std.rpc", respond_name, SYNTHETIC)?;
		let resp = self.emit_let(
			Rvalue::CallClosure(respond, vec![req.clone(), thunk]),
			SYNTHETIC,
		);
		self.push_stmt(StmtKind::Let(result, Rvalue::Use(resp)), SYNTHETIC);
		Ok(())
	}

	/// `std.rpc.server-origin`, a baked build-time constant: a thunk returning the
	/// `--server-url` string (empty = same-origin). Never settable at runtime.
	fn synthesize_server_origin(&mut self) -> Result<FuncId, String> {
		let origin = self.compiler.rpc_base_url.clone().unwrap_or_default();
		self.push_scope(format!("{}.server-origin@thunk", self.current_module), &[]);
		self.push_stmt(StmtKind::Return(Atom::Const(Const::Str(origin))), SYNTHETIC);
		let scope = self.scopes.pop().unwrap();
		Ok(self.add_function(finish_scope(scope)))
	}

	// ---- trait instances / constrained defs ----------------------------

	/// Lower a trait `instance` def to its method-dictionary global. A build
	/// failure poisons the slot.
	fn lower_instance(&mut self, instance: &compiler::ast::InstanceNode) {
		let gid = match instance.instance_slot_name.rsplit_once('.') {
			Some((m, n)) => self.globals.lookup(m, n),
			None => None,
		};
		let Some(gid) = gid else { return };
		// An instance whose methods are all `built-in "tag"` bodies lowers to a
		// `PreEval::MethodDict` of builtin members — the same representation the
		// prelude's primitive instances (`numeric int`, `ord int`, …) use. The
		// wasm backend wraps each internal builtin into a dict-slot closure;
		// internal builtins (`int-add`, …) aren't host imports, so they can't go
		// through the runtime `MakeDict` path. Only concrete instances qualify
		// (the MethodDict wrapper requires every member be a wrappable builtin).
		if instance.where_clause.is_empty() {
			if let Some(members) = builtin_method_dict_members(instance) {
				self
					.globals
					.set_pre_evaluated(gid, PreEval::MethodDict(members));
				return;
			}
		}
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
	/// closure.
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
				));
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
	/// and forwards to the underlying global with the dicts prepended.
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
			poll_fn: None,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(g_var, Rvalue::GlobalRef(global))),
				Stmt::synthetic(StmtKind::Let(
					r_var,
					Rvalue::CallClosure(Atom::Var(g_var), call_args),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(r_var))),
			]),
			var_reprs: Vec::new(),
			param_reprs: vec![Repr::Boxed; n as usize],
			ret_repr: Repr::Boxed,
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

	/// A bare builtin (`to-string`, `print`, `math.sqrt`, …) referenced as a
	/// first-class *value*: wrap it in a closure of the builtin's arity that
	/// forwards to a direct builtin call. A builtin has no standalone runtime
	/// value on the deploy backend (it isn't a `$closure`), so without this it
	/// couldn't be passed to a higher-order function — this makes it indistinguishable
	/// from a user function. Mirrors `lower_constrained_value_ref` (minus dict
	/// captures); WasmGC resolves the inner call to a direct builtin op, so the
	/// wrapper is treated as an ordinary closure.
	fn lower_builtin_value_ref(
		&mut self,
		global: GlobalId,
		tag: &str,
		arity: u32,
		range: Range,
	) -> Atom {
		let params: Vec<VarId> = (0..arity).map(VarId).collect();
		let g_var = VarId(arity);
		let r_var = VarId(arity + 1);
		let call_args: Vec<Atom> = params.iter().map(|v| Atom::Var(*v)).collect();
		let wrapper = Function {
			name: format!("{}.{}@builtin-value", self.current_module, tag),
			module: self.current_module.clone(),
			params,
			captures: Vec::new(),
			is_async: false,
			poll_fn: None,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(g_var, Rvalue::GlobalRef(global))),
				Stmt::synthetic(StmtKind::Let(
					r_var,
					Rvalue::CallClosure(Atom::Var(g_var), call_args),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(r_var))),
			]),
			var_reprs: Vec::new(),
			param_reprs: vec![Repr::Boxed; arity as usize],
			ret_repr: Repr::Boxed,
		};
		let wrapper_fid = self.add_function(wrapper);
		self.emit_let(Rvalue::MakeClosure(wrapper_fid, Vec::new()), range)
	}

	/// If `expr` (an identifier or `module.value` in *value* position) names a bare
	/// builtin, lower it to a forwarding closure (`lower_builtin_value_ref`).
	/// Returns `None` for non-builtins (locals, user globals, variants), so the
	/// caller falls back to the ordinary `GlobalRef` path.
	fn try_builtin_value_ref(
		&mut self,
		expr: &ExprNode,
		range: Range,
	) -> Result<Option<Atom>, String> {
		// Arity comes from the reference's (instantiated) function type — the same
		// source `lower_constrained_value_ref` uses. A non-function type can't be a
		// callable builtin reference.
		let arity = match &expr.ty {
			Type::Fun(params, _) => params.len() as u32,
			_ => return Ok(None),
		};
		let Some(g) = self.ref_global(expr) else {
			return Ok(None);
		};
		let Some(tag) = self.globals.builtin_tag(g).map(str::to_string) else {
			return Ok(None);
		};
		Ok(Some(self.lower_builtin_value_ref(g, &tag, arity, range)))
	}

	/// The global an identifier / `module.value` reference resolves to, if any
	/// (locals and variants yield `None`). Read-only resolution used to classify a
	/// value-position reference; never emits.
	fn ref_global(&mut self, expr: &ExprNode) -> Option<GlobalId> {
		match &expr.kind {
			ExprKind::Identifier(id) => match self.resolve(&id.name) {
				Ok(Resolved::Global(g)) => Some(g),
				_ => None,
			},
			ExprKind::NamespaceAccess(path) => match path.as_slice() {
				[head, tail] => {
					if head.name.contains('.') {
						return self.globals.lookup(&head.name, &tail.name);
					}
					let qualified_module = self.imports.get(&head.name).cloned()?;
					self.globals.lookup(&qualified_module, &tail.name)
				}
				_ => None,
			},
			_ => None,
		}
	}

	/// Lower an expression in *callee* position. A bare global/builtin reference
	/// stays an un-wrapped `GlobalRef` here (a direct call), where in value
	/// position `lower_expr` would wrap a builtin into a forwarding closure.
	/// Dispatch / constrained-value refs and every other expr lower as usual.
	fn lower_callee(&mut self, expr: &ExprNode) -> Result<Atom, String> {
		if expr.trait_dispatch.is_none() && undrained_dispatch_cells(expr).is_none() {
			match &expr.kind {
				ExprKind::Identifier(id) => return self.lower_identifier(&id.name, expr.range),
				ExprKind::NamespaceAccess(path) => return self.lower_namespace(path, expr.range),
				_ => {}
			}
		}
		self.lower_expr(expr)
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
				// A bare builtin used as a value forwards through a closure.
				if let Some(atom) = self.try_builtin_value_ref(expr, range)? {
					return Ok(atom);
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
			ExprKind::RecordUpdate { base, fields } => {
				let base_atom = self.lower_expr(base)?;
				let mut ir_fields = Vec::with_capacity(fields.len());
				for (name, value) in fields {
					let atom = self.lower_expr(value)?;
					ir_fields.push((name.name.clone(), atom));
				}
				Ok(self.emit_let(
					Rvalue::RecordUpdate {
						base: base_atom,
						fields: ir_fields,
					},
					range,
				))
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
				let shape = record_shape_of(&receiver.ty);
				Ok(self.emit_let(Rvalue::GetField(recv, field.name.clone(), shape), range))
			}
			ExprKind::ElementAccess { receiver, index } => {
				let recv = self.lower_expr(receiver)?;
				Ok(self.emit_let(Rvalue::GetElement(recv, *index as u32), range))
			}
			ExprKind::NamespaceAccess(path) => {
				if let Some(cell) = &expr.trait_dispatch {
					return self.lower_dispatch(cell, range);
				}
				if let Some(cells) = undrained_dispatch_cells(expr) {
					return self.lower_constrained_value_ref(expr, &cells);
				}
				// A bare builtin used as a value (`math.sqrt`) forwards through a closure.
				if let Some(atom) = self.try_builtin_value_ref(expr, range)? {
					return Ok(atom);
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
			ExprKind::Regex(node) => self.lower_regex_pattern(node, range),
			ExprKind::Defer(inner) => self.lower_defer(inner, range),
			ExprKind::Try(node) => self.lower_try(node, range),
			ExprKind::Scope(node) => self.lower_scope(node, range),
			// A `using` block is a transparent scope; lower its body as a statement
			// sequence whose value is the last expression. (The leading-dot members
			// were rewritten to `NamespaceAccess` during analysis.)
			ExprKind::Using { body, .. } => self.lower_body(body),
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
				// `std.task.or-else`), resolved directly against globals.
				if head.name.contains('.') {
					if let Some(g) = self.globals.lookup(&head.name, &tail.name) {
						return Ok(self.emit_let(Rvalue::GlobalRef(g), range));
					}
					return Err(format!("`{}.{}` not found", head.name, tail.name));
				}
				// `module.value` — an imported function/value (e.g. `point.distance`).
				if let Some(qualified_module) = self.imports.get(&head.name).cloned() {
					if let Some(g) = self.globals.lookup(&qualified_module, &tail.name) {
						return Ok(self.emit_let(Rvalue::GlobalRef(g), range));
					}
				}
				// `Enum.variant` where `head` is an enum in scope unqualified: a
				// local-module enum (`color.red`), a prelude enum (`ordering.gt`),
				// or the eponymous enum of an imported module (`point.cartesian`,
				// where `use geometry.point` binds `point` to `geometry.point.point`
				// — named after the module's last segment, so an alias still works).
				// Tried in the analyzer's type-scope precedence: a local or prelude
				// declaration shadows the import. Each candidate falls through if it
				// lacks the variant, so a shadowed name still resolves.
				let current = self.current_module.clone();
				let mut candidates = vec![
					format!("{}.{}", current, head.name),
					format!("__prelude__.{}", head.name),
				];
				if let Some(qualified_module) = self.imports.get(&head.name) {
					let eponym = qualified_module
						.rsplit('.')
						.next()
						.unwrap_or(qualified_module);
					candidates.push(format!("{}.{}", qualified_module, eponym));
				}
				for qualified_enum in candidates {
					if self.enums.contains_key(&qualified_enum) {
						if let Ok(arity) = self.variant_arity(&qualified_enum, &tail.name) {
							return self.make_variant_ref(&qualified_enum, &tail.name, arity, range);
						}
					}
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

	/// Resolve a pattern variant's discriminant tag within its (known) enum,
	/// erroring if the enum or variant is unknown. Used at pattern lowering so
	/// the IR carries the tag and the emitter never re-derives it by name.
	fn pattern_variant_tag(&self, enum_name: &str, variant: &str) -> Result<u32, String> {
		self
			.variant_tag(enum_name, variant)
			.ok_or_else(|| format!("unknown variant `{variant}` of `{enum_name}`"))
	}

	/// The qualified enum a constructor pattern belongs to. The top-level subject
	/// carries a concrete enum type, but nested sub-patterns are lowered with
	/// `Type::Unknown` (the analyzer threads their types but lowering doesn't), so
	/// fall back to the source qualification on the head — `module.enum.variant`,
	/// `enum.variant`, or a bare prelude variant — mirroring `lower_namespace`.
	fn resolve_variant_enum(
		&self,
		head: &compiler::ast::ConstructorHead,
		subject_ty: &Type,
	) -> Result<String, String> {
		if let Type::Enum(qualified, _) = subject_ty {
			return Ok(qualified.clone());
		}
		let variant = &head.variant.name;
		// `module.enum.variant`: qualify the enum through the import.
		if let (Some(module), Some(enum_name)) = (&head.module, &head.enum_name) {
			let qualified_module = self
				.imports
				.get(&module.name)
				.ok_or_else(|| format!("`{}` is not an imported module", module.name))?;
			return Ok(format!("{qualified_module}.{}", enum_name.name));
		}
		// `enum.variant`: a local-module enum, a prelude enum, or the eponymous
		// enum of an imported module — same precedence as `lower_namespace`.
		if let Some(enum_name) = &head.enum_name {
			let mut candidates = vec![
				format!("{}.{}", self.current_module, enum_name.name),
				format!("__prelude__.{}", enum_name.name),
			];
			if let Some(qualified_module) = self.imports.get(&enum_name.name) {
				let eponym = qualified_module
					.rsplit('.')
					.next()
					.unwrap_or(qualified_module);
				candidates.push(format!("{qualified_module}.{eponym}"));
			}
			return candidates
				.into_iter()
				.find(|q| self.variant_tag(q, variant).is_some())
				.ok_or_else(|| format!("variant `{variant}` of `{}` not found", enum_name.name));
		}
		// Bare head: a prelude variant (`some`/`none`/`ok`/`err`/...).
		self
			.enums
			.keys()
			.filter(|k| k.starts_with("__prelude__."))
			.find(|k| self.variant_tag(k, variant).is_some())
			.cloned()
			.ok_or_else(|| format!("bare variant `{variant}` is not a prelude variant"))
	}

	fn lower_call(&mut self, call: &compiler::ast::CallNode, range: Range) -> Result<Atom, String> {
		if let Some(result) = self.try_lower_wire_call(call, range) {
			return result;
		}
		let callee = self.lower_callee(&call.callee)?;
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
					// emit the direct `BinOp` (`AddInt`, `DivFloat`, …) instead of a
					// boxed dispatch through the method dictionary. Each `BinOp` is
					// behavior-identical to the dict's builtin method (`int-add` ≡
					// `AddInt`, …; `DivInt` matches `int-div`), so this is
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
		// `x |> f a b` pipes `x` in as `f`'s first argument.
		if let Operator::Chain = op {
			return self.lower_chain(left, right, range);
		}
		// Concrete, non-dispatched operator: a direct `BinOp` picked by
		// operand type. Evaluate left then right (matching `emit.rs`).
		let is_float = matches!(left.ty, Type::Float) || matches!(right.ty, Type::Float);
		// `==`/`!=` on concrete numbers devirtualize to the unboxed `EqI64`/`NeF64`/…
		// (else they'd box both operands for the structural `__eq` helper); anything
		// else (strings, records, bools, polymorphic) keeps structural `Eq`/`Ne`.
		let binop = concrete_eq_binop(op, &left.ty, &right.ty)
			.or_else(|| binop_for(op, is_float))
			.ok_or("unsupported binary operator")?;
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
	/// extracted from it.
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
	/// `Resolved` shapes:
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
			// The `wire` "dictionary" is a schema value, not a method dict:
			// build the `__prelude__.wire-schema` tree from the shape.
			DispatchTarget::WireSchema(shape) => self.lower_wire_shape(shape, range),
		}
	}

	/// Build a `__prelude__.wire-schema` value from a compile-time `WireShape`.
	/// This is the runtime reification of a `wire a` dictionary, consumed by the
	/// `wire-encode` / `wire-decode` builtins. A `Var` leaf splices in a
	/// forwarded `wire a` dict (itself a `wire-schema` value).
	fn lower_wire_shape(
		&mut self,
		shape: &compiler::ast::WireShape,
		range: Range,
	) -> Result<Atom, String> {
		use compiler::ast::WireShape as W;
		const E: &str = "__prelude__.wire-schema";
		let str_atom = |s: &str| Atom::Const(Const::Str(s.to_string()));
		match shape {
			W::Int => self.make_variant(E, "s-int", vec![], range),
			W::Float => self.make_variant(E, "s-float", vec![], range),
			W::Bool => self.make_variant(E, "s-bool", vec![], range),
			W::Str => self.make_variant(E, "s-string", vec![], range),
			W::Bytes => self.make_variant(E, "s-bytes", vec![], range),
			W::Duration => self.make_variant(E, "s-duration", vec![], range),
			W::Nothing => self.make_variant(E, "s-nothing", vec![], range),
			W::List(inner) => {
				let i = self.lower_wire_shape(inner, range)?;
				self.make_variant(E, "s-list", vec![i], range)
			}
			W::Dict(key, value) => {
				let k = self.lower_wire_shape(key, range)?;
				let v = self.lower_wire_shape(value, range)?;
				self.make_variant(E, "s-dict", vec![k, v], range)
			}
			W::Tuple(shapes) => {
				let mut items = Vec::with_capacity(shapes.len());
				for s in shapes {
					items.push(ListItem::Elem(self.lower_wire_shape(s, range)?));
				}
				let list = self.emit_let(Rvalue::MakeList(items), range);
				self.make_variant(E, "s-tuple", vec![list], range)
			}
			W::Record(fields) => {
				let mut items = Vec::with_capacity(fields.len());
				for (name, sh) in fields {
					let sa = self.lower_wire_shape(sh, range)?;
					let pair = self.emit_let(Rvalue::MakeTuple(vec![str_atom(name), sa]), range);
					items.push(ListItem::Elem(pair));
				}
				let list = self.emit_let(Rvalue::MakeList(items), range);
				self.make_variant(E, "s-record", vec![list], range)
			}
			W::Enum {
				qualified,
				variants,
			} => {
				let mut items = Vec::with_capacity(variants.len());
				for (vname, field_shapes) in variants {
					let mut field_items = Vec::with_capacity(field_shapes.len());
					for fs in field_shapes {
						field_items.push(ListItem::Elem(self.lower_wire_shape(fs, range)?));
					}
					let fields_list = self.emit_let(Rvalue::MakeList(field_items), range);
					let pair = self.emit_let(Rvalue::MakeTuple(vec![str_atom(vname), fields_list]), range);
					items.push(ListItem::Elem(pair));
				}
				let vlist = self.emit_let(Rvalue::MakeList(items), range);
				self.make_variant(E, "s-enum", vec![str_atom(qualified), vlist], range)
			}
			W::EnumRef(qualified) => {
				let q = Atom::Const(Const::Str(qualified.clone()));
				self.make_variant(E, "s-enum-ref", vec![q], range)
			}
			W::Var(resolved) => self.lower_dict_atom(resolved, range),
		}
	}

	/// Lower a backtick regex literal to a `__prelude__.regex-pattern` enum
	/// value tree — the shape the pure-Pluma `std.regex` engine walks. Mirrors
	/// `lower_wire_shape`: a structured AST node reified as a runtime value
	/// rather than flattened to a string. The quantifier `RegexKind`s all
	/// collapse to `p-repeat inner min max` (`max = -1` is unbounded); a
	/// `Grouping` is transparent (its inner node, no wrapper).
	fn lower_regex_pattern(&mut self, node: &RegexNode, range: Range) -> Result<Atom, String> {
		use RegexAnchor as A;
		use RegexKind as K;
		const E: &str = "__prelude__.regex-pattern";
		let str_atom = |s: &str| Atom::Const(Const::Str(s.to_string()));
		match &node.kind {
			K::Literal(s) => {
				let bytes = Atom::Const(Const::Bytes(s.clone().into_bytes()));
				self.make_variant(E, "p-literal", vec![bytes], range)
			}
			K::CharacterClass(c) => self.make_variant(E, "p-class", vec![str_atom(c)], range),
			K::Anchor(a) => {
				let name = match a {
					A::Start => "start",
					A::End => "end",
					A::Boundary => "boundary",
				};
				self.make_variant(E, "p-anchor", vec![str_atom(name)], range)
			}
			K::OneOrMore(inner) => self.lower_repeat(inner, 1, -1, range),
			K::ZeroOrMore(inner) => self.lower_repeat(inner, 0, -1, range),
			K::OneOrZero(inner) => self.lower_repeat(inner, 0, 1, range),
			K::ExactCount(inner, n) => self.lower_repeat(inner, *n as i64, *n as i64, range),
			K::AtLeastCount(inner, n) => self.lower_repeat(inner, *n as i64, -1, range),
			K::AtMostCount(inner, n) => self.lower_repeat(inner, 0, *n as i64, range),
			K::RangeCount(inner, min, max) => self.lower_repeat(inner, *min as i64, *max as i64, range),
			K::Grouping(inner) => self.lower_regex_pattern(inner, range),
			K::Sequence(parts) => {
				let list = self.lower_regex_list(parts, range)?;
				self.make_variant(E, "p-sequence", vec![list], range)
			}
			K::Alternation(parts) => {
				let list = self.lower_regex_list(parts, range)?;
				self.make_variant(E, "p-alternation", vec![list], range)
			}
			K::NamedCapture(name, inner) => {
				let i = self.lower_regex_pattern(inner, range)?;
				self.make_variant(E, "p-capture", vec![str_atom(name), i], range)
			}
		}
	}

	fn lower_repeat(
		&mut self,
		inner: &RegexNode,
		lo: i64,
		hi: i64,
		range: Range,
	) -> Result<Atom, String> {
		let i = self.lower_regex_pattern(inner, range)?;
		self.make_variant(
			"__prelude__.regex-pattern",
			"p-repeat",
			vec![i, Atom::Const(Const::Int(lo)), Atom::Const(Const::Int(hi))],
			range,
		)
	}

	fn lower_regex_list(&mut self, parts: &[RegexNode], range: Range) -> Result<Atom, String> {
		let mut items = Vec::with_capacity(parts.len());
		for p in parts {
			items.push(ListItem::Elem(self.lower_regex_pattern(p, range)?));
		}
		Ok(self.emit_let(Rvalue::MakeList(items), range))
	}

	/// If `call` is a `wire` trait-method call (`encode x` / `decode b`),
	/// lower it to the corresponding builtin applied to the schema dict + the
	/// user args, and return `Some(result)`. Otherwise `None` (a normal call).
	/// The schema (the resolved `wire a` dictionary) is passed as the builtin's
	/// first argument — `wire`'s methods aren't read out of a method dict.
	fn try_lower_wire_call(
		&mut self,
		call: &compiler::ast::CallNode,
		range: Range,
	) -> Option<Result<Atom, String>> {
		let cell = call.callee.trait_dispatch.as_ref()?;
		let (tag, resolved) = {
			let b = cell.borrow();
			if b.trait_name != "wire" {
				return None;
			}
			let tag = match b.method_idx {
				Some(0) => "wire-encode",
				Some(1) => "wire-decode",
				Some(2) => "wire-fingerprint",
				_ => return None,
			};
			match b.resolved.clone() {
				Some(r) => (tag, r),
				None => return Some(Err("unresolved wire dispatch".to_string())),
			}
		};
		Some((|| {
			let gid = self
				.globals
				.lookup("__prelude__", tag)
				.ok_or("wire builtin global not registered")?;
			let builtin = self.emit_let(Rvalue::GlobalRef(gid), range);
			let mut args = Vec::with_capacity(1 + call.args.len());
			args.push(self.lower_dict_atom(&resolved, range)?);
			for a in &call.args {
				args.push(self.lower_expr(a)?);
			}
			Ok(self.emit_let(Rvalue::CallClosure(builtin, args), range))
		})())
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
		let callee_atom = self.lower_callee(callee)?;
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
						let tag = self.pattern_variant_tag(qualified, &id.name)?;
						return Ok(Pattern::Variant {
							variant: id.name.clone(),
							tag,
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
			PatternKind::Constructor(head, subs) => {
				// Resolve the variant's enum (and thus its discriminant tag) here,
				// where the type is known, and carry the tag in the IR. The emitter
				// then compares tags directly — it never re-derives a tag from the
				// bare name, which would be ambiguous when two enums share a variant
				// name (e.g. `pending` in both `run-status` and the synthetic
				// `__poll`).
				let qualified = self.resolve_variant_enum(head, subject_ty)?;
				let tag = self.pattern_variant_tag(&qualified, &head.variant.name)?;
				let fields = self.lower_sub_patterns(subs)?;
				Ok(Pattern::Variant {
					variant: head.variant.name.clone(),
					tag,
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
				// The closed shape comes from the subject's type at this site; nested
				// record sub-patterns are lowered with `Type::Unknown` (below), so they
				// get `None` and flow uniform — matching the receiver-type threading in
				// `FieldAccess`.
				let shape = record_shape_of(subject_ty);
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
					shape,
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
		let param_reprs: Vec<Repr> = fun
			.params
			.iter()
			.map(|p| crate::repr::repr_of_type(&p.ty))
			.collect();
		let fn_name = format!(
			"{}.fun@{}:{}",
			self.current_module, fun.range.start.line, fun.range.start.col
		);
		self.lower_closure(fn_name, &param_names, &param_reprs, &fun.body, range)
	}

	/// Lower a closure body into its own `Function` and return a `MakeClosure`
	/// atom for it. Shared by `fun` literals, `defer` thunks, and `scope` body
	/// closures. A task `try` anywhere in the body marks the new function
	/// `is_async` (via `lower_try`); the async-lowering pass later turns such a
	/// function into a poll-driven `$task` (see `Function::is_async`).
	fn lower_closure(
		&mut self,
		fn_name: String,
		param_names: &[&str],
		param_reprs: &[Repr],
		body: &[ExprNode],
		outer_range: Range,
	) -> Result<Atom, String> {
		self.push_scope(fn_name, param_names);
		// Record the function's signature reprs (the projection of the AST param
		// types and the body's tail type) onto the `Function`. They stay all-`Boxed`
		// under the uniform-boxed contract (see `Function::param_reprs`).
		if let Some(scope) = self.scopes.last_mut() {
			scope.param_reprs = param_reprs.to_vec();
			scope.ret_repr = body
				.last()
				.map(|e| crate::repr::repr_of_type(&e.ty))
				.unwrap_or(Repr::Boxed);
		}
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
	/// evaluates to `nothing`. The cleanup stack is walked LIFO at `Return` (and
	/// on `try`-failure short-circuit).
	fn lower_defer(&mut self, inner: &ExprNode, range: Range) -> Result<Atom, String> {
		let fn_name = format!(
			"{}.defer@{}:{}",
			self.current_module, inner.range.start.line, inner.range.start.col
		);
		let closure = self.lower_closure(fn_name, &[], &[], std::slice::from_ref(inner), range)?;
		self.push_stmt(StmtKind::PushDefer(closure), range);
		Ok(Atom::Const(Const::Unit))
	}

	/// A task-carrier `try Pattern = value` and its continuation (`rest`). Lowers
	/// to: evaluate the awaited task, `Await` it (suspend), bind the pattern, then
	/// lower the continuation inline. Sets the enclosing function `is_async` — the
	/// single async marker; `ir::cps` turns the `Await`-bearing body into a poll fn.
	/// `option`/`result` `try`s are rewritten to `<carrier>.then` calls by the
	/// analyzer and never reach here.
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
	/// `std.task.scope-new` kernel: `scope-new <manual> (fun handle { body })`.
	/// The body becomes its own closure frame (so its `try`s suspend within the
	/// scope's child fiber, not this one — that's why a `scope` doesn't make the
	/// enclosing function async). Mirrors `emit.rs`'s `emit_scope`.
	fn lower_scope(&mut self, node: &ScopeNode, range: Range) -> Result<Atom, String> {
		let g = self
			.globals
			.lookup("std.task", "scope-new")
			.ok_or("`std.task.scope-new` not found")?;
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
		let body = self.lower_closure(fn_name, &[handle_name], &[Repr::Boxed], &node.body, range)?;
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
	/// closure callee (the tail call reuses the frame) and live for a
	/// builtin/ctor/async-fn callee (which ignores the tail flag).
	fn lower_call_tail(
		&mut self,
		call: &compiler::ast::CallNode,
		range: Range,
	) -> Result<(), String> {
		// A `wire` method call lowers to a builtin call, not a tail-callable
		// closure — emit it then return its result.
		if let Some(result) = self.try_lower_wire_call(call, range) {
			let atom = result?;
			self.push_stmt(StmtKind::Return(atom), range);
			return Ok(());
		}
		let callee = self.lower_callee(&call.callee)?;
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
			param_reprs: vec![Repr::Boxed; param_names.len()],
			ret_repr: Repr::Boxed,
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
	/// as needed.
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
					return Some(self.add_capture(scope_idx, name, CaptureSrc::ParentLocal(pv)));
				}
				Some(ScopeSlot::Capture(pi)) => {
					return Some(self.add_capture(scope_idx, name, CaptureSrc::ParentCapture(pi)));
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

	fn build_entry(&mut self, test_suites: &[(String, GlobalId)]) -> Result<FuncId, String> {
		// The entry drives one root to completion, then returns its result. The
		// only choice is *what* root: a `pluma test` runner over the discovered
		// suites, or the module's `main`. Everything downstream — the scheduler
		// wrapper, sync/async tolerance — is shared, so this is the single place
		// the two programs diverge.
		let mut body: Vec<Stmt> = Vec::new();
		let mut next: u32 = 0;
		let result = match self.test_color {
			Some(color) => self.emit_test_runner(&mut body, &mut next, test_suites, color)?,
			None => self.emit_main_call(&mut body, &mut next, test_suites)?,
		};
		body.push(Stmt::synthetic(StmtKind::Return(result)));
		Ok(self.wrap_entry(body))
	}

	/// The `pluma run` root: load `main` and invoke it with unit. On a client emit
	/// with `remote def`s, install the web transport first (the build's one bit of
	/// injected setup, so app code configures nothing). A suite-bearing module with
	/// no `main` (a `*.test.pa` reached outside test mode) drives nothing.
	fn emit_main_call(
		&mut self,
		body: &mut Vec<Stmt>,
		next: &mut u32,
		test_suites: &[(String, GlobalId)],
	) -> Result<Atom, String> {
		let main_module = match &self.entry_override {
			Some(m) => m.clone(),
			None => self
				.compiler
				.entry_modules
				.first()
				.ok_or("no entry module")?
				.clone(),
		};
		let main = match self.globals.lookup(&main_module, "main") {
			Some(g) => g,
			None if !test_suites.is_empty() => return Ok(Atom::Const(Const::Unit)),
			None => return Err(format!("module `{}` has no `main` def", main_module)),
		};
		if self.is_client_emit() && !self.compiler.rpc_endpoints.is_empty() {
			if let Some(install) = self.globals.lookup("std.rpc.web", "install") {
				let g = fresh_let(body, next, Rvalue::GlobalRef(install));
				body.push(Stmt::synthetic(StmtKind::Discard(Rvalue::CallClosure(
					Atom::Var(g),
					vec![Atom::Const(Const::Unit)],
				))));
			}
		}
		let mainref = fresh_let(body, next, Rvalue::GlobalRef(main));
		let result = fresh_let(
			body,
			next,
			Rvalue::CallClosure(Atom::Var(mainref), vec![Atom::Const(Const::Unit)]),
		);
		Ok(Atom::Var(result))
	}

	/// The `pluma test` root: build a `list {name, tests}` from the discovered
	/// suites and call `std.test.run-all color suites`. The suites are referenced
	/// by `GlobalId`, so their privacy (a `*.test.pa`'s `tests` is private) doesn't
	/// matter — no source-level import is involved.
	fn emit_test_runner(
		&mut self,
		body: &mut Vec<Stmt>,
		next: &mut u32,
		test_suites: &[(String, GlobalId)],
		color: bool,
	) -> Result<Atom, String> {
		let run_all = self
			.globals
			.lookup("std.test", "run-all")
			.ok_or("`std.test.run-all` was not compiled — does a `*.test.pa` file `use std.test`?")?;

		// One `{name, tests}` record per suite. Field order is name-sorted to
		// match the record-shape layout the backends expect from `MakeRecord`.
		let mut items: Vec<ListItem> = Vec::new();
		for (module, gid) in test_suites {
			let display = module.strip_suffix(".test").unwrap_or(module).to_string();
			let tests = fresh_let(body, next, Rvalue::GlobalRef(*gid));
			let rec = fresh_let(
				body,
				next,
				Rvalue::MakeRecord(vec![
					("name".to_string(), Atom::Const(Const::Str(display))),
					("tests".to_string(), Atom::Var(tests)),
				]),
			);
			items.push(ListItem::Elem(Atom::Var(rec)));
		}

		let list = fresh_let(body, next, Rvalue::MakeList(items));
		let runner = fresh_let(body, next, Rvalue::GlobalRef(run_all));
		let result = fresh_let(
			body,
			next,
			Rvalue::CallClosure(
				Atom::Var(runner),
				vec![Atom::Const(Const::Bool(color)), Atom::Var(list)],
			),
		);
		Ok(Atom::Var(result))
	}

	/// Wrap a synthesized entry body in the `__entry__` function. Zero params, no
	/// captures, not async (the scheduler wrapper tolerates a task or plain value
	/// the body returns) — the single shape every entry takes.
	fn wrap_entry(&mut self, body: Vec<Stmt>) -> FuncId {
		let func = Function {
			name: "__entry__".to_string(),
			module: String::new(),
			params: Vec::new(),
			captures: Vec::new(),
			is_async: false,
			poll_fn: None,
			body: Block(body),
			var_reprs: Vec::new(),
			param_reprs: Vec::new(),
			ret_repr: Repr::Boxed,
		};
		self.add_function(func)
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
			poll_fn: None,
			body: Block(vec![Stmt::synthetic(StmtKind::Return(Atom::Const(
				Const::Unit,
			)))]),
			var_reprs: Vec::new(),
			param_reprs: Vec::new(),
			ret_repr: Repr::Boxed,
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
		poll_fn: None,
		body: Block(scope.stmts),
		// Filled in by a single pass over all functions at the end of `run`.
		var_reprs: Vec::new(),
		param_reprs: scope.param_reprs,
		ret_repr: scope.ret_repr,
	}
}

/// Append a synthetic `let v = rv` to a flat statement list and return the fresh
/// `VarId`, bumping the caller's counter. Used by the entry synthesizers, which
/// build a straight-line body outside the normal scope machinery.
fn fresh_let(body: &mut Vec<Stmt>, next: &mut u32, rv: Rvalue) -> VarId {
	let v = VarId(*next);
	*next += 1;
	body.push(Stmt::synthetic(StmtKind::Let(v, rv)));
	v
}

/// Build the module's local-namespace -> qualified-module map: explicit `use`
/// declarations plus the auto-imported modules (unless shadowed).
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
/// absent.
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
/// captures through nested closures).
fn synthetic_dict_name(slot: u16) -> String {
	format!("__dict_{}__", slot)
}

/// If every method body of a concrete instance is a `built-in "tag"`, return the
/// dict members (in canonical/trait order) as `PreEval::Builtin`s — ready to be
/// the instance's `PreEval::MethodDict` global, mirroring the prelude's primitive
/// instances. `None` if any method is a normal body (the runtime `MakeDict` path
/// handles those) or there are no methods. Member return repr is `Boxed`: a
/// method-dict slot is called through the uniform dict ABI, so the wasm wrapper
/// boxes the result regardless (same as `seed_prelude_globals`).
fn builtin_method_dict_members(instance: &compiler::ast::InstanceNode) -> Option<Vec<PreEval>> {
	let mut by_name: HashMap<&str, &ExprNode> = HashMap::new();
	for m in &instance.methods {
		if let DefinitionKind::Expr(e) = &m.kind {
			by_name.insert(m.name.name.as_str(), e);
		}
	}
	if instance.canonical_method_order.is_empty() {
		return None;
	}
	let mut members = Vec::with_capacity(instance.canonical_method_order.len());
	for method_name in &instance.canonical_method_order {
		let expr = by_name.get(method_name.as_str()).copied()?;
		match &expr.kind {
			ExprKind::Builtin(tag) => members.push(PreEval::Builtin(tag.clone(), Repr::Boxed)),
			_ => return None,
		}
	}
	Some(members)
}

/// If both operands of a `numeric`-dispatched arithmetic operator are the *same
/// concrete* numeric type (`int` or `float`), return the direct `BinOp` so the
/// dispatch can be devirtualized. Returns `None` when either
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
/// `BinOp` (`LtI64`/`LeF64`/…) so it lowers to a relational op rather than the
/// `ord.compare … {==,!=} variant` desugaring. For concrete floats this
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

/// If a `==`/`!=` has concrete numeric operands, return the direct unboxed
/// `BinOp` (`EqI64`/`NeF64`/…), so the comparison reads `i64`/`f64` registers
/// instead of boxing both sides for the structural `__eq` helper. Behavior-
/// identical: int equality is i64 equality, and concrete float `==`/`!=` is
/// IEEE (`nan != nan`) — exactly the semantics structural `==`/`!=` already has
/// on floats (and distinct, like `concrete_ord_binop`, from the total-order
/// `ord.compare`). Non-numeric or polymorphic operands return `None` (keep the
/// structural `Eq`/`Ne`, which still covers strings/records/bools/enums).
fn concrete_eq_binop(op: &Operator, left: &Type, right: &Type) -> Option<BinOp> {
	let is_float = match (left, right) {
		(Type::Int, Type::Int) => false,
		(Type::Float, Type::Float) => true,
		_ => return None,
	};
	Some(match (op, is_float) {
		(Operator::Equality, false) => BinOp::EqI64,
		(Operator::Equality, true) => BinOp::EqF64,
		(Operator::Inequality, false) => BinOp::NeI64,
		(Operator::Inequality, true) => BinOp::NeF64,
		_ => return None,
	})
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

/// The closed-record shape of `ty`, if it is one: a `Type::Record` with a `None`
/// tail (exactly these fields). The field names are returned name-sorted, the
/// same canonical order `MakeRecord` lays out its `names`/`values` arrays, so a
/// field's index in `RecordShape::fields` is its runtime slot. An open record
/// (`Some` tail — a row-polymorphic position) or any non-record type yields
/// `None`, leaving the value on the uniform self-describing representation.
fn record_shape_of(ty: &Type) -> Option<RecordShape> {
	match ty {
		Type::Record(fields, None) => {
			let mut names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
			names.sort();
			Some(RecordShape { fields: names })
		}
		_ => None,
	}
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
/// variants table. Run over all modules (including the prelude, which defines
/// `option`/`result`/`ordering`).
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
	/// start `Reserved`.
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

	/// The builtin tag a global holds, if it's a bare `built-in` value (not a
	/// method dict or a user def thunk). Lets a *value*-position reference to a
	/// builtin be wrapped into a forwarding closure so it behaves like any user
	/// function passed by name.
	fn builtin_tag(&self, id: GlobalId) -> Option<&str> {
		match &self.slots[id.0 as usize] {
			Slot::PreEvaluated(PreEval::Builtin(tag, _)) => Some(tag),
			_ => None,
		}
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
/// method order matches each trait's declaration order. Built as backend-neutral
/// `PreEval` values.
fn seed_prelude_globals(g: &mut GlobalTable) {
	// Prelude builtins (print/debug/to-string/wire-*) return strings/bytes/nothing,
	// and method-dict members are never resolved as direct builtin calls — so a
	// `Boxed` return repr is correct for all of them. The scalar-returning builtins
	// that benefit from an unboxed result (`bytes-get`, `bytes-length`, …) are
	// `.pa` defs, seeded with their real declared repr in `lower_value_def`.
	let builtin = |tag: &str| PreEval::Builtin(tag.to_string(), Repr::Boxed);

	g.add_pre_evaluated("__prelude__", "print", builtin("print"));
	g.add_pre_evaluated("__prelude__", "debug", builtin("debug"));
	g.add_pre_evaluated("__prelude__", "to-string", builtin("to-string"));
	// `wire` codec builtins: a `wire` method call loads one of
	// these as its callee, passing the schema dict as the first argument.
	g.add_pre_evaluated("__prelude__", "wire-encode", builtin("wire-encode"));
	g.add_pre_evaluated("__prelude__", "wire-decode", builtin("wire-decode"));
	g.add_pre_evaluated(
		"__prelude__",
		"wire-fingerprint",
		builtin("wire-fingerprint"),
	);

	// The `numeric`/`ord`/`hash` instance dicts on the primitives are no longer
	// seeded here: they're written in `prelude.pa` as `implement … { def add =
	// built-in "int-add" }` and lower to the identical `PreEval::MethodDict`
	// through the ordinary all-builtin-instance path (`lower_instance`).
}

/// Reserve a slot for each user-module top-level value def, alias (its
/// constructor), and trait instance (its method dictionary). Enums and trait
/// declarations are types, not values, so they get no slot.
fn reserve_user_globals(g: &mut GlobalTable, compiler: &Compiler) {
	for (module_name, module) in &compiler.modules {
		let Some(ast) = &module.ast else { continue };
		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(expr) => {
					let gid = g.reserve(module_name, &def.name.name);
					// Pre-evaluate `built-in "tag"` defs to `PreEval::Builtin` here, in the
					// reservation pre-pass — *before* any module body is lowered — so a
					// value-position reference to a builtin from another (earlier-lowered)
					// module sees it's a builtin and wraps it into a forwarding closure.
					// (`lower_value_def` re-sets this identically when the def is lowered.)
					if let ExprKind::Builtin(tag) = &expr.kind {
						// The RPC builtins are synthesized as thunks at lower time, not
						// host imports — leave their slots unset so `lower_value_def`
						// fills them (mirrors a normal `def f = fun …`).
						if tag == "rpc-dispatch" || tag == "rpc-server-origin" {
							continue;
						}
						let ret = match &expr.ty {
							Type::Fun(_, ret) => crate::repr::repr_of_type(ret),
							_ => Repr::Boxed,
						};
						g.set_pre_evaluated(gid, PreEval::Builtin(tag.clone(), ret));
					}
				}
				DefinitionKind::Alias(_) => {
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
		// `numeric`/`ord`/`hash` instance dicts are no longer seeded here — they
		// live in `prelude.pa` and lower through the ordinary instance path (see
		// `seed_prelude_globals`), so they're not in this seed-only table.
		assert!(g.lookup("__prelude__", "numeric@int").is_none());

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
		let p = g.add_pre_evaluated("m", "print", PreEval::Builtin("print".into(), Repr::Boxed));
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
			GlobalInit::PreEvaluated(PreEval::Builtin(..))
		));
		assert!(matches!(globals[1], GlobalInit::Thunk(FuncId(7))));
	}
}
