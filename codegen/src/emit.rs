// AST → bytecode lowering.
//
// Single-pass walk of all loaded modules + the prelude. The CodeGen struct
// owns the in-progress Program and tracks per-function scope while compiling
// expressions. Identifier resolution distinguishes locals, captures, and
// globals; closures capture their free vars explicitly.

use compiler::ast::{
	CallNode, DefinitionKind, ExprKind, ExprNode, FunNode, FunParamNode, IdentifierNode, IfNode,
	LetNode, ListItem, LiteralKind, ModuleNode, Operator, PatternKind, PatternNode, RegexAnchor,
	RegexKind, RegexNode, ScopeNode, TryNode, WhenNode, WhileNode,
};
use compiler::types::Type;
use compiler::Range;
use std::collections::HashMap;
use std::rc::Rc;
use vm::{native_modules, Function, GlobalIdx, Instruction, Program, RegexData, SlotIdx, Value};

pub fn compile(compiler: &compiler::Compiler) -> Result<Program, String> {
	let mut cg = CodeGen::new();

	// Prelude: `print`, `debug`, and `to-string` as pre-evaluated globals.
	cg.add_evaluated_global("__prelude__", "print", Value::Builtin(Rc::from("print")));
	cg.add_evaluated_global("__prelude__", "debug", Value::Builtin(Rc::from("debug")));
	cg.add_evaluated_global(
		"__prelude__",
		"to-string",
		Value::Builtin(Rc::from("to-string")),
	);

	// Prelude trait instance dictionaries. Each instance is a positional
	// array of method values keyed by trait declaration order (`numeric`
	// is `add, sub, mul, div, negate`). Codegen reads these globals when
	// a class-constraint discharge resolves to `Resolved::Global(name)`.
	cg.add_evaluated_global(
		"__prelude__",
		"numeric@int",
		Value::MethodDict(Rc::new(vec![
			Value::Builtin(Rc::from("int-add")),
			Value::Builtin(Rc::from("int-sub")),
			Value::Builtin(Rc::from("int-mul")),
			Value::Builtin(Rc::from("int-div")),
			Value::Builtin(Rc::from("int-negate")),
		])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"numeric@float",
		Value::MethodDict(Rc::new(vec![
			Value::Builtin(Rc::from("float-add")),
			Value::Builtin(Rc::from("float-sub")),
			Value::Builtin(Rc::from("float-mul")),
			Value::Builtin(Rc::from("float-div")),
			Value::Builtin(Rc::from("float-negate")),
		])),
	);
	// `ord` trait: one method (`compare`), three concrete instances.
	cg.add_evaluated_global(
		"__prelude__",
		"ord@int",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("int-compare"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"ord@float",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("float-compare"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"ord@string",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("string-compare"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"ord@bytes",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("bytes-compare"))])),
	);
	// `hash` trait: one method (`hash`), four concrete instances.
	cg.add_evaluated_global(
		"__prelude__",
		"hash@int",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("int-hash"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"hash@float",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("float-hash"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"hash@string",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("string-hash"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"hash@bytes",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("bytes-hash"))])),
	);
	cg.add_evaluated_global(
		"__prelude__",
		"hash@bool",
		Value::MethodDict(Rc::new(vec![Value::Builtin(Rc::from("bool-hash"))])),
	);

	// Prelude enums (`option`, `result`, `ordering`) are declared in
	// `compiler/src/prelude.pa` — `collect_enum_defs` (below) picks them
	// up from the prelude module's AST the same way it does any other
	// module's enums.

	// Native modules: each def's value is a pre-evaluated Builtin, each
	// constant's is its concrete Value.
	for module in native_modules() {
		for def in &module.defs {
			cg.add_evaluated_global(
				module.name,
				def.name,
				Value::Builtin(Rc::from(def.builtin_tag)),
			);
		}
		for c in module.constants {
			cg.add_evaluated_global(module.name, c.name, c.value);
		}
	}

	// Collect enum defs from every loaded module so pattern matching can
	// disambiguate bare identifier patterns against the subject's variants.
	for (module_name, module) in &compiler.modules {
		if let Some(ast) = &module.ast {
			collect_enum_defs(module_name, ast, &mut cg.enum_variants);
		}
	}

	// First pass: allocate a global slot per top-level value def (and per
	// alias's constructor) in every user module. No code yet — just slot
	// indices, so later expression-codegen can resolve forward references.
	for (module_name, module) in &compiler.modules {
		if let Some(ast) = &module.ast {
			for def in &ast.body {
				match &def.kind {
					DefinitionKind::Expr(_) | DefinitionKind::Alias(_) => {
						cg.reserve_global(module_name, &def.name.name);
					}
					DefinitionKind::Enum(_) => {
						// Enums aren't values; nothing to allocate as a global.
					}
					DefinitionKind::Trait(_) => {
						// Trait declarations are types, not values — nothing
						// to allocate.
					}
					DefinitionKind::Instance(instance_node) => {
						// Each concrete instance gets one global slot holding
						// its `Value::MethodDict` of methods. The slot name was
						// chosen by the analyzer (`<module>.<trait>@<head>`).
						let (module, name) = match instance_node.instance_slot_name.rsplit_once('.') {
							Some((m, n)) => (m, n),
							None => continue,
						};
						cg.reserve_global(module, name);
					}
				}
			}
		}
	}

	// Second pass: emit a thunk function for each top-level def and store
	// its index in the global's Pending state. Aliases get a constructor
	// thunk (curried as a single-arg function that returns its arg
	// unchanged — aliases are transparent in this language).
	for (module_name, module) in &compiler.modules {
		if let Some(ast) = &module.ast {
			cg.compile_module(module_name, ast)?;
		}
	}

	// Discover test suites: every entry module that exports `def tests`
	// (a `core.testing` suite). `pluma test` runs these; the runner also
	// needs `core.testing.new` to build the registrar it threads in.
	for module_name in &compiler.entry_modules {
		if let Some(idx) = cg.lookup_global(module_name, "tests") {
			cg.program.test_suites.push((module_name.clone(), idx));
		}
	}
	cg.program.test_new = cg.lookup_global("core.testing", "new");

	// Build the entry function: load main, call it with (), return.
	// If there's no `main` but the program defines test suites, emit a
	// no-op entry — `pluma test` skips invoking it and goes straight to
	// the suites, and `pluma run` of a test-only file errors clearly.
	let primary_entry = compiler.entry_modules.first();
	let main_global = primary_entry.and_then(|name| cg.lookup_global(name, "main"));
	let entry_idx = match main_global {
		Some(idx) => cg.emit_entry_function(idx),
		None if !cg.program.test_suites.is_empty() => cg.emit_noop_entry_function(),
		None => {
			let name = primary_entry.map(String::as_str).unwrap_or("<none>");
			return Err(format!("module `{}` has no `main` def", name));
		}
	};
	cg.program.entry = entry_idx;

	Ok(cg.program)
}

// --------------------------------------------------------------------------
// CodeGen state.
// --------------------------------------------------------------------------

struct CodeGen {
	program: Program,
	// Constants pool dedup.
	const_lookup: HashMap<String, u32>,
	// Bytes pool dedup. Keyed by the raw byte sequence; identical literals
	// share an entry.
	bytes_lookup: HashMap<Vec<u8>, u32>,
	// (module, def_name) -> GlobalIdx
	global_lookup: HashMap<(String, String), u32>,
	// Per-module enums: qualified_enum -> [(variant_name, arity)]
	enum_variants: HashMap<String, Vec<(String, usize)>>,
}

impl CodeGen {
	fn new() -> Self {
		Self {
			program: Program {
				functions: Vec::new(),
				constants: Vec::new(),
				bytes_constants: Vec::new(),
				regex_patterns: Vec::new(),
				globals: Vec::new(),
				field_lists: Vec::new(),
				global_by_name: HashMap::new(),
				enum_variants: HashMap::new(),
				entry: 0,
				test_suites: Vec::new(),
				test_new: None,
			},
			const_lookup: HashMap::new(),
			bytes_lookup: HashMap::new(),
			global_lookup: HashMap::new(),
			enum_variants: HashMap::new(),
		}
	}

	fn intern(&mut self, s: &str) -> u32 {
		if let Some(&idx) = self.const_lookup.get(s) {
			return idx;
		}
		let idx = self.program.constants.len() as u32;
		self.program.constants.push(Rc::new(s.to_string()));
		self.const_lookup.insert(s.to_string(), idx);
		idx
	}

	fn intern_bytes(&mut self, b: &[u8]) -> u32 {
		if let Some(&idx) = self.bytes_lookup.get(b) {
			return idx;
		}
		let idx = self.program.bytes_constants.len() as u32;
		self.program.bytes_constants.push(Rc::new(b.to_vec()));
		self.bytes_lookup.insert(b.to_vec(), idx);
		idx
	}

	fn intern_field_list(&mut self, fields: Vec<u32>) -> u32 {
		// No dedup for now — record shapes are rarely repeated, and the
		// lookup cost would offset the savings. Revisit if profiling shows
		// many duplicate lists.
		let idx = self.program.field_lists.len() as u32;
		self.program.field_lists.push(fields);
		idx
	}

	fn lookup_global(&self, module: &str, name: &str) -> Option<u32> {
		self
			.global_lookup
			.get(&(module.to_string(), name.to_string()))
			.copied()
	}

	fn reserve_global(&mut self, module: &str, name: &str) -> u32 {
		let key = (module.to_string(), name.to_string());
		if let Some(&idx) = self.global_lookup.get(&key) {
			return idx;
		}
		let idx = self.program.globals.len() as u32;
		self
			.program
			.globals
			.push(vm::program::GlobalSlot::Pending(0));
		self.global_lookup.insert(key.clone(), idx);
		self.program.global_by_name.insert(key, idx);
		idx
	}

	fn add_evaluated_global(&mut self, module: &str, name: &str, value: Value) -> u32 {
		let idx = self.reserve_global(module, name);
		self.program.globals[idx as usize] = vm::program::GlobalSlot::Evaluated(value);
		idx
	}

	fn set_global_thunk(&mut self, idx: u32, fn_idx: u32) {
		self.program.globals[idx as usize] = vm::program::GlobalSlot::Pending(fn_idx);
	}

	fn compile_module(&mut self, module_name: &str, ast: &ModuleNode) -> Result<(), String> {
		// Build the module's imports map (local_name -> qualified_module).
		// Mirrors the analyzer's view: explicit `use` declarations, plus
		// any auto-imported native modules that the user didn't shadow.
		let mut imports: HashMap<String, String> = ast
			.uses
			.iter()
			.map(|u| (u.local_name().name.clone(), u.module_name()))
			.collect();
		for (full_name, local_name) in compiler::AUTO_IMPORTS {
			imports
				.entry(local_name.to_string())
				.or_insert_with(|| full_name.to_string());
		}

		// Stash enum_variants from this module's enum defs into the Program.
		// (Already collected into self.enum_variants; flush into program for
		// the VM to use at runtime if needed.)
		for (k, v) in &self.enum_variants {
			self.program.enum_variants.insert(k.clone(), v.clone());
		}

		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(expr) => {
					let global_idx = self
						.lookup_global(module_name, &def.name.name)
						.expect("global slot reserved in pass 1");
					// `built-in "tag"` as the RHS: stash the tag and drop a
					// `Value::Builtin` straight into the global slot — no
					// thunk, no function. Validity isn't checked here;
					// `built-in` is internal to the stdlib and builtin.rs's
					// fallthrough arm produces a runtime error if a tag
					// has no matching handler.
					if let ExprKind::Builtin(tag) = &expr.kind {
						self.program.globals[global_idx as usize] =
							vm::program::GlobalSlot::Evaluated(Value::Builtin(Rc::from(tag.as_str())));
						continue;
					}
					let fn_idx = if def.dict_param_count > 0 {
						// Constrained def: emit a thunk whose body is a Fun
						// with K extra leading dict params at slots 0..K-1.
						self.compile_constrained_thunk(
							module_name,
							&imports,
							&format!("{}.{}", module_name, def.name.name),
							expr,
							def.dict_param_count,
						)?
					} else {
						self.compile_thunk(
							module_name,
							&imports,
							&format!("{}.{}", module_name, def.name.name),
							expr,
						)?
					};
					self.set_global_thunk(global_idx, fn_idx);
				}
				DefinitionKind::Alias(_) => {
					// Alias constructor: `fun x { x }`. Single-arg pass-through.
					let global_idx = self
						.lookup_global(module_name, &def.name.name)
						.expect("global slot reserved in pass 1");
					let alias_fn_idx = self.emit_alias_constructor(module_name, &def.name.name);
					// The "thunk" returns a closure over alias_fn_idx with no captures.
					let thunk_idx = self.emit_alias_thunk(module_name, &def.name.name, alias_fn_idx);
					self.set_global_thunk(global_idx, thunk_idx);
				}
				DefinitionKind::Enum(_) => {
					// Nothing to emit — enums show up at use sites as
					// MakeVariant or MakeVariantCtor based on their variant
					// shape, looked up via self.enum_variants.
				}
				DefinitionKind::Trait(_) => {
					// Trait declarations are types, not values.
				}
				DefinitionKind::Instance(instance_node) => {
					self.compile_instance(module_name, &imports, instance_node)?;
				}
			}
		}

		Ok(())
	}

	// A thunk for a def with class constraints. The def body must be a Fun
	// expression (constraints only quantify functions). The compiled
	// function takes K hidden dict params at slots 0..K-1 followed by the
	// user-facing params at K..K+N-1. The thunk builds and returns a
	// closure over this function.
	fn compile_constrained_thunk(
		&mut self,
		current_module: &str,
		imports: &HashMap<String, String>,
		name: &str,
		expr: &ExprNode,
		dict_param_count: u16,
	) -> Result<u32, String> {
		// Constraints only make sense on function defs. If the body isn't a
		// Fun, refuse — Phase 4 may diagnose this earlier; for now, fail
		// loudly so we don't generate broken bytecode.
		let (user_params, body, body_range) = match &expr.kind {
			ExprKind::Fun(FunNode {
				params,
				body,
				range,
				..
			}) => (params, body, *range),
			_ => {
				return Err(format!(
					"codegen: constrained def `{}` must have a function body",
					name
				))
			}
		};

		let total_arity = dict_param_count + user_params.len() as u16;
		let mut inner_fb =
			FunctionBuilder::new(name.to_string(), current_module.to_string(), total_arity);
		let mut inner_scope = Scope::new();
		// Register synthetic dict locals at slots 0..K-1 under
		// `__dict_<n>__` names so nested closures' `Forwarded(n)` cells
		// resolve through the normal scope/capture machinery.
		for n in 0..dict_param_count {
			inner_scope.define_local(&synthetic_dict_name(n), n);
		}
		// User-facing params live at slots K..K+N-1.
		for (i, p) in user_params.iter().enumerate() {
			inner_scope.define_local(&p.ident.name, dict_param_count + i as u16);
		}

		let mut parent_scopes: Vec<*mut Scope> = Vec::new();
		let res = (|| -> Result<(), String> {
			if body.is_empty() {
				inner_fb.emit(Instruction::LoadNothing, body_range);
			} else {
				for (i, e) in body.iter().enumerate() {
					let is_last = i == body.len() - 1;
					emit_expr_with_parents(
						self,
						current_module,
						imports,
						&mut inner_fb,
						&mut inner_scope,
						&mut parent_scopes,
						e,
						is_last,
					)?;
					if !is_last {
						inner_fb.emit(Instruction::Pop, e.range);
					}
				}
			}
			inner_fb.emit(Instruction::Return, body_range);
			Ok(())
		})();
		res?;
		inner_fb.capture_count = 0;
		let inner_fn_idx = self.add_function(inner_fb);

		// Thunk: build a closure of `inner_fn_idx` (no captures) and
		// return it as the global's value.
		let mut thunk_fb =
			FunctionBuilder::new(format!("{}@thunk", name), current_module.to_string(), 0);
		thunk_fb.emit(
			Instruction::MakeClosure {
				fn_idx: inner_fn_idx,
				num_captures: 0,
			},
			body_range,
		);
		thunk_fb.emit(Instruction::Return, body_range);
		Ok(self.add_function(thunk_fb))
	}

	// A thunk function: zero arity, no captures, body is `expr` compiled
	// followed by Return. The expression is in tail position because its
	// result is the thunk's return value.
	fn compile_thunk(
		&mut self,
		current_module: &str,
		imports: &HashMap<String, String>,
		name: &str,
		expr: &ExprNode,
	) -> Result<u32, String> {
		let mut fb = FunctionBuilder::new(name.to_string(), current_module.to_string(), 0);
		let mut scope = Scope::new();
		emit_expr(
			self,
			current_module,
			imports,
			&mut fb,
			&mut scope,
			expr,
			true,
		)?;
		fb.emit(Instruction::Return, expr.range);
		Ok(self.add_function(fb))
	}

	// Compile a user instance — concrete (no inner dicts) or parametric
	// (instance constructor takes inner dicts as args).
	//
	// **Concrete** (`instance.where_clause` empty): each method is a
	// 0-arity thunk returning its closure; a builder thunk Call()s each,
	// then `MakeDict`s the results. Global slot holds the resulting Dict.
	//
	// **Parametric** (`instance.where_clause` non-empty): build a single
	// constructor function of arity K = where_clause.len(). Inside, the
	// inner dicts are bound at slots 0..K-1 under synthetic
	// `__dict_<n>__` names, and each method is emitted as a *nested*
	// `Fun` so it captures whatever inner dicts it actually uses. The
	// global slot holds a closure of this constructor; call sites with
	// `Resolved::InstanceChain` call it with the inner dicts to receive
	// a fresh `Value::MethodDict`.
	fn compile_instance(
		&mut self,
		module_name: &str,
		imports: &HashMap<String, String>,
		instance: &compiler::ast::InstanceNode,
	) -> Result<(), String> {
		let (module, slot_name) = match instance.instance_slot_name.rsplit_once('.') {
			Some(p) => p,
			None => {
				return Err(format!(
					"codegen: malformed instance slot name `{}`",
					instance.instance_slot_name
				))
			}
		};
		let global_idx = self
			.lookup_global(module, slot_name)
			.expect("instance global slot reserved in pass 1");

		if instance.where_clause.is_empty() {
			// Concrete: each method is its own 0-arity thunk; the dict
			// builder Call()s them and bundles the results.
			let mut method_fn_indices: HashMap<String, u32> = HashMap::new();
			for method in &instance.methods {
				let expr = match &method.kind {
					DefinitionKind::Expr(e) => e,
					_ => continue,
				};
				let qualified = format!("{}.{}#{}", module, slot_name, method.name.name);
				let fn_idx = self.compile_thunk(module_name, imports, &qualified, expr)?;
				method_fn_indices.insert(method.name.name.clone(), fn_idx);
			}

			let thunk_name = format!("{}@dict-builder", instance.instance_slot_name);
			let mut thunk_fb = FunctionBuilder::new(thunk_name, module_name.to_string(), 0);
			for method_name in &instance.canonical_method_order {
				let fn_idx = match method_fn_indices.get(method_name) {
					Some(idx) => *idx,
					None => {
						return Err(format!(
							"codegen: instance `{}` is missing method `{}`",
							instance.instance_slot_name, method_name
						))
					}
				};
				thunk_fb.emit(
					Instruction::MakeClosure {
						fn_idx,
						num_captures: 0,
					},
					instance.range,
				);
				thunk_fb.emit(Instruction::Call(0), instance.range);
			}
			thunk_fb.emit(
				Instruction::MakeDict(instance.canonical_method_order.len() as u16),
				instance.range,
			);
			thunk_fb.emit(Instruction::Return, instance.range);
			let thunk_idx = self.add_function(thunk_fb);
			self.set_global_thunk(global_idx, thunk_idx);
			return Ok(());
		}

		// Parametric: build the instance constructor function.
		let where_count = instance.where_clause.len() as u16;
		let ctor_name = format!("{}@ctor", instance.instance_slot_name);
		let mut ctor_fb = FunctionBuilder::new(ctor_name, module_name.to_string(), where_count);
		let mut ctor_scope = Scope::new();
		for n in 0..where_count {
			ctor_scope.define_local(&synthetic_dict_name(n), n);
		}

		// Index methods by name so we can emit them in canonical order.
		let mut methods_by_name: HashMap<&str, &ExprNode> = HashMap::new();
		for method in &instance.methods {
			if let DefinitionKind::Expr(e) = &method.kind {
				methods_by_name.insert(method.name.name.as_str(), e);
			}
		}

		let mut parent_scopes: Vec<*mut Scope> = Vec::new();
		for method_name in &instance.canonical_method_order {
			let method_expr = match methods_by_name.get(method_name.as_str()) {
				Some(e) => *e,
				None => {
					return Err(format!(
						"codegen: instance `{}` is missing method `{}`",
						instance.instance_slot_name, method_name
					))
				}
			};
			// Method bodies are Fun expressions. Emit each as a nested
			// closure inside the constructor — the existing capture path
			// hoists references to the synthetic dict locals
			// automatically.
			let (params, body, range) = match &method_expr.kind {
				ExprKind::Fun(FunNode {
					params,
					body,
					range,
					..
				}) => (params.as_slice(), body.as_slice(), *range),
				_ => {
					return Err(format!(
						"codegen: method `{}` on instance `{}` is not a function",
						method_name, instance.instance_slot_name
					))
				}
			};
			emit_fun(
				self,
				module_name,
				imports,
				&mut ctor_fb,
				&mut ctor_scope,
				&mut parent_scopes,
				params,
				body,
				range,
			)?;
		}
		ctor_fb.emit(
			Instruction::MakeDict(instance.canonical_method_order.len() as u16),
			instance.range,
		);
		ctor_fb.emit(Instruction::Return, instance.range);
		let ctor_idx = self.add_function(ctor_fb);

		// Wrap the constructor in a 0-arity thunk that returns a closure of
		// it. The global slot then holds the closure, ready to be called
		// with the inner dicts at `InstanceChain` call sites.
		let mut builder_fb = FunctionBuilder::new(
			format!("{}@builder", instance.instance_slot_name),
			module_name.to_string(),
			0,
		);
		builder_fb.emit(
			Instruction::MakeClosure {
				fn_idx: ctor_idx,
				num_captures: 0,
			},
			instance.range,
		);
		builder_fb.emit(Instruction::Return, instance.range);
		let builder_idx = self.add_function(builder_fb);
		self.set_global_thunk(global_idx, builder_idx);

		Ok(())
	}

	fn emit_alias_constructor(&mut self, module: &str, alias_name: &str) -> u32 {
		let mut fb = FunctionBuilder::new(format!("alias-ctor:{}", alias_name), module.to_string(), 1);
		fb.slot_count = 1;
		fb.emit(Instruction::LoadLocal(0), Range::collapsed(0, 0));
		fb.emit(Instruction::Return, Range::collapsed(0, 0));
		self.add_function(fb)
	}

	fn emit_alias_thunk(&mut self, module: &str, alias_name: &str, alias_fn_idx: u32) -> u32 {
		let mut fb = FunctionBuilder::new(format!("alias-thunk:{}", alias_name), module.to_string(), 0);
		fb.emit(
			Instruction::MakeClosure {
				fn_idx: alias_fn_idx,
				num_captures: 0,
			},
			Range::collapsed(0, 0),
		);
		fb.emit(Instruction::Return, Range::collapsed(0, 0));
		self.add_function(fb)
	}

	fn emit_entry_function(&mut self, main_global: u32) -> u32 {
		let mut fb = FunctionBuilder::new("__entry__".into(), String::new(), 0);
		fb.emit(Instruction::LoadGlobal(main_global), Range::collapsed(0, 0));
		fb.emit(Instruction::LoadNothing, Range::collapsed(0, 0));
		// Tail-call so main runs in our frame rather than nested under it.
		fb.emit(Instruction::TailCall(1), Range::collapsed(0, 0));
		fb.emit(Instruction::Return, Range::collapsed(0, 0));
		self.add_function(fb)
	}

	// Entry function used for test-only programs: push `nothing` and
	// return. The VM's `run()` path can still complete; the test runner
	// then drives each test directly via `call_test`.
	fn emit_noop_entry_function(&mut self) -> u32 {
		let mut fb = FunctionBuilder::new("__entry__".into(), String::new(), 0);
		fb.emit(Instruction::LoadNothing, Range::collapsed(0, 0));
		fb.emit(Instruction::Return, Range::collapsed(0, 0));
		self.add_function(fb)
	}

	fn add_function(&mut self, fb: FunctionBuilder) -> u32 {
		let idx = self.program.functions.len() as u32;
		self.program.functions.push(Function {
			name: fb.name,
			module: fb.module,
			param_count: fb.param_count,
			slot_count: fb.slot_count,
			capture_count: fb.capture_count,
			body: fb.body,
			source_ranges: fb.source_ranges,
		});
		idx
	}
}

// --------------------------------------------------------------------------
// Per-function scope (locals + captures).
// --------------------------------------------------------------------------

struct FunctionBuilder {
	name: String,
	module: String,
	param_count: u16,
	slot_count: u16,
	capture_count: u16,
	body: Vec<Instruction>,
	source_ranges: Vec<Range>,
}

impl FunctionBuilder {
	fn new(name: String, module: String, param_count: u16) -> Self {
		Self {
			name,
			module,
			param_count,
			slot_count: param_count,
			capture_count: 0,
			body: Vec::new(),
			source_ranges: Vec::new(),
		}
	}

	fn emit(&mut self, instr: Instruction, range: Range) -> u32 {
		let idx = self.body.len() as u32;
		self.body.push(instr);
		self.source_ranges.push(range);
		idx
	}

	fn patch_jump(&mut self, idx: u32, target: u32) {
		match &mut self.body[idx as usize] {
			Instruction::Jump(o) | Instruction::JumpIfFalse(o) => *o = target,
			Instruction::MatchInt(_, o)
			| Instruction::MatchFloat(_, o)
			| Instruction::MatchDuration(_, o)
			| Instruction::MatchString(_, o)
			| Instruction::MatchBytes(_, o)
			| Instruction::MatchBool(_, o)
			| Instruction::MatchNothing(o)
			| Instruction::MatchVariant { on_fail: o, .. }
			| Instruction::MatchTuple { on_fail: o, .. }
			| Instruction::MatchRecord { on_fail: o, .. }
			| Instruction::MatchList { on_fail: o, .. } => *o = target,
			other => panic!("patch_jump: not a jump-like instruction: {:?}", other),
		}
	}

	fn here(&self) -> u32 {
		self.body.len() as u32
	}

	fn alloc_slot(&mut self) -> SlotIdx {
		let s = self.slot_count;
		self.slot_count += 1;
		s
	}
}

// A scope maps source names to either a local slot or a capture index. When
// the codegen descends into a nested `fun`, it builds a new Scope chained to
// the enclosing one for free-var lookups.
struct Scope {
	// Slots in the current function for locals (params + lets).
	locals: Vec<(String, SlotIdx)>,
	// Captures recorded so far in the current function's closure (each
	// resolves to an expression that loads from the *parent* scope).
	captures: Vec<Capture>,
	// How many `let` shadowings deep we are — used so that resolution finds
	// the most recently bound name first.
	scope_marks: Vec<usize>,
}

#[derive(Clone)]
struct Capture {
	name: String,
	// How to push the captured value onto the stack in the *enclosing*
	// scope when building the closure.
	source: CaptureSource,
}

#[derive(Clone)]
enum CaptureSource {
	Local(SlotIdx),
	Capture(u16),
}

impl Scope {
	fn new() -> Self {
		Self {
			locals: Vec::new(),
			captures: Vec::new(),
			scope_marks: vec![0],
		}
	}

	fn define_local(&mut self, name: &str, slot: SlotIdx) {
		self.locals.push((name.to_string(), slot));
	}

	fn enter(&mut self) {
		self.scope_marks.push(self.locals.len());
	}

	fn leave(&mut self) {
		let mark = self.scope_marks.pop().unwrap_or(0);
		self.locals.truncate(mark);
	}

	fn resolve_local(&self, name: &str) -> Option<SlotIdx> {
		for (n, s) in self.locals.iter().rev() {
			if n == name {
				return Some(*s);
			}
		}
		None
	}

	fn resolve_capture(&self, name: &str) -> Option<u16> {
		for (i, c) in self.captures.iter().enumerate() {
			if c.name == name {
				return Some(i as u16);
			}
		}
		None
	}
}

// --------------------------------------------------------------------------
// Expression emission.
// --------------------------------------------------------------------------

// Result of resolving an identifier in the current scope chain.
#[allow(dead_code)]
enum Resolution {
	Local(SlotIdx),
	Capture(u16),
	Global(GlobalIdx),
	// `enum_name` itself (not a value, but a namespace for variant access).
	EnumName(String),
	// `imported_module` — same idea, namespace.
	Imported(String),
	// A bare variant constructor reference (`some 5` / `none`) — qualified
	// enum + variant name + arity. Resolved when no value binding matches
	// and the variant name is unique across known enums.
	BareVariant(String, String, usize),
}

fn resolve_identifier(
	cg: &CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	scope: &mut Scope,
	parent_scopes: &mut [&mut Scope],
	name: &str,
) -> Option<Resolution> {
	if let Some(slot) = scope.resolve_local(name) {
		return Some(Resolution::Local(slot));
	}
	if let Some(idx) = scope.resolve_capture(name) {
		return Some(Resolution::Capture(idx));
	}
	// Free var: try to capture from parent scopes (innermost-first).
	if !parent_scopes.is_empty() {
		let parent_idx = parent_scopes.len() - 1;
		// Look in the immediate parent. If found there, add a capture
		// pointing at the parent's local or capture. If not, recurse —
		// each intermediate scope captures from its parent.
		let mut found_source: Option<CaptureSource> = None;
		{
			let parent = &mut *parent_scopes[parent_idx];
			if let Some(slot) = parent.resolve_local(name) {
				found_source = Some(CaptureSource::Local(slot));
			} else if let Some(cap) = parent.resolve_capture(name) {
				found_source = Some(CaptureSource::Capture(cap));
			}
		}
		if found_source.is_none() {
			// Recurse: pretend we're in the parent, looking further up.
			let (head, tail) = parent_scopes.split_at_mut(parent_idx);
			let parent: &mut Scope = tail[0];
			if let Some(res) = resolve_identifier(cg, current_module, imports, parent, head, name) {
				match res {
					Resolution::Local(slot) => {
						found_source = Some(CaptureSource::Local(slot));
					}
					Resolution::Capture(cap) => {
						found_source = Some(CaptureSource::Capture(cap));
					}
					// Globals / namespaces don't need to be captured — they
					// can be loaded directly at the inner site.
					other => return Some(other),
				}
			}
		}
		if let Some(source) = found_source {
			let cap_idx = scope.captures.len() as u16;
			scope.captures.push(Capture {
				name: name.to_string(),
				source,
			});
			return Some(Resolution::Capture(cap_idx));
		}
	}
	// Global in this module?
	if let Some(idx) = cg.lookup_global(current_module, name) {
		return Some(Resolution::Global(idx));
	}
	// Prelude (synthetic module)?
	if let Some(idx) = cg.lookup_global("__prelude__", name) {
		return Some(Resolution::Global(idx));
	}
	// An imported module name?
	if let Some(qualified) = imports.get(name) {
		return Some(Resolution::Imported(qualified.clone()));
	}
	// An enum name in the current module?
	let qualified_enum = format!("{}.{}", current_module, name);
	if cg.enum_variants.contains_key(&qualified_enum) {
		return Some(Resolution::EnumName(qualified_enum));
	}
	// A bare variant constructor — `some` instead of `option.some`. Local-
	// module enums take precedence over imported/prelude variants with the
	// same name (mirrors the analyzer's `disambiguate_variant_matches`).
	let local_prefix = format!("{}.", current_module);
	let mut local_match = None;
	let mut other_match = None;
	for (qualified, variants) in &cg.enum_variants {
		for (variant, arity) in variants {
			if variant == name {
				let resolved = Resolution::BareVariant(qualified.clone(), variant.clone(), *arity);
				if qualified.starts_with(&local_prefix) {
					local_match = Some(resolved);
				} else if other_match.is_none() {
					other_match = Some(resolved);
				}
			}
		}
	}
	local_match.or(other_match)
}

fn emit_expr(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	expr: &ExprNode,
	tail: bool,
) -> Result<(), String> {
	emit_expr_with_parents(
		cg,
		current_module,
		imports,
		fb,
		scope,
		&mut Vec::new(),
		expr,
		tail,
	)
}

// `tail` is true when the expression's result is about to be Return'd directly
// (without further computation). Used to convert Call -> TailCall, which the
// VM treats as a frame swap rather than a frame push.
fn emit_expr_with_parents(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	expr: &ExprNode,
	tail: bool,
) -> Result<(), String> {
	let range = expr.range;
	match &expr.kind {
		ExprKind::Literal(lit) => emit_literal_with_cg(cg, fb, &lit.kind, range),
		ExprKind::EmptyTuple => {
			fb.emit(Instruction::LoadNothing, range);
		}
		ExprKind::Identifier(ident) => {
			// Bare trait method reference: `hash 42` instead of `hash.hash 42`.
			// The analyzer already attached the dispatch cell — emit a
			// dispatch load and skip the regular identifier resolution.
			if let Some(cell) = &expr.trait_dispatch {
				emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
			} else if let Some(cells) = undrained_dispatch_cells(expr) {
				// A constrained function referenced as a first-class value
				// (not a direct callee, so its dict sink was never drained
				// into a Call's dict_args). Wrap it so the dicts are
				// prepended — see `emit_constrained_value_ref`.
				emit_constrained_value_ref(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					expr,
					&cells,
					range,
				)?;
			} else {
				emit_identifier(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					ident,
					range,
				)?;
			}
		}
		ExprKind::Grouping(inner) => emit_expr_with_parents(
			cg,
			current_module,
			imports,
			fb,
			scope,
			parent_scopes,
			inner,
			tail,
		)?,
		ExprKind::Let(LetNode { pattern, value, .. }) => {
			// Value is stored into the local; the `let` expression's own
			// result is Nothing — so the value is never in tail position.
			emit_expr_with_parents(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				value,
				false,
			)?;
			match &pattern.kind {
				PatternKind::Identifier(ident) => {
					let slot = fb.alloc_slot();
					fb.emit(Instruction::StoreLocal(slot), range);
					scope.define_local(&ident.name, slot);
				}
				_ => {
					// Reuse the pattern matcher used by `if`/`when`/`while`.
					// The analyzer guarantees an irrefutable pattern here, so
					// the returned fail jumps are unreachable at runtime — we
					// patch them to the let's exit (where LoadNothing runs)
					// to keep the bytecode well-formed.
					let subject_ty = value.ty.clone();
					let fail_idx = emit_pattern(cg, fb, scope, &subject_ty, pattern)?;
					let exit_target = fb.here();
					for fi in fail_idx {
						fb.patch_jump(fi, exit_target);
					}
				}
			}
			fb.emit(Instruction::LoadNothing, range);
		}
		ExprKind::Defer(inner) => {
			// Lower `defer expr` to a zero-arg cleanup thunk (`fun { expr }`)
			// pushed onto the running frame's cleanup stack via PushDefer; the
			// VM walks that stack LIFO at Return. The thunk captures whatever
			// locals `expr` references by value at the point the `defer`
			// executes — matching Go's "arguments evaluated at defer time"
			// (immaterial in Pluma, where values are immutable). The `defer`
			// expression itself evaluates to `nothing`.
			emit_fun(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				&[],
				std::slice::from_ref(inner.as_ref()),
				range,
			)?;
			fb.emit(Instruction::PushDefer, range);
			fb.emit(Instruction::LoadNothing, range);
		}
		ExprKind::Tuple(elems) => {
			for e in elems {
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					e,
					false,
				)?;
			}
			fb.emit(Instruction::MakeTuple(elems.len() as u16), range);
		}
		ExprKind::List(items) => {
			if !items.iter().any(|it| it.is_spread()) {
				// Common case, no spreads: identical lowering to before.
				for item in items {
					emit_expr_with_parents(
						cg,
						current_module,
						imports,
						fb,
						scope,
						parent_scopes,
						item.expr(),
						false,
					)?;
				}
				fb.emit(Instruction::MakeList(items.len() as u16), range);
			} else if items.len() == 1 {
				// `[...xs]` — the spread is already the whole list.
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					items[0].expr(),
					false,
				)?;
			} else {
				// Build each segment as a list, then concatenate them. A run of
				// consecutive plain items collapses into one `MakeList`; each
				// spread expr is itself a list and forms its own segment.
				let mut segments: u16 = 0;
				let mut run: u16 = 0;
				for item in items {
					match item {
						ListItem::Item(e) => {
							emit_expr_with_parents(
								cg,
								current_module,
								imports,
								fb,
								scope,
								parent_scopes,
								e,
								false,
							)?;
							run += 1;
						}
						ListItem::Spread(e) => {
							if run > 0 {
								fb.emit(Instruction::MakeList(run), range);
								segments += 1;
								run = 0;
							}
							emit_expr_with_parents(
								cg,
								current_module,
								imports,
								fb,
								scope,
								parent_scopes,
								e,
								false,
							)?;
							segments += 1;
						}
					}
				}
				if run > 0 {
					fb.emit(Instruction::MakeList(run), range);
					segments += 1;
				}
				fb.emit(Instruction::ConcatLists(segments), range);
			}
		}
		ExprKind::Record(fields) => {
			let mut field_idxs = Vec::with_capacity(fields.len());
			for (field_name, field_value) in fields {
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					field_value,
					false,
				)?;
				field_idxs.push(cg.intern(&field_name.name));
			}
			let fields_idx = cg.intern_field_list(field_idxs);
			fb.emit(Instruction::MakeRecord(fields_idx), range);
		}
		ExprKind::Interpolation(parts) => {
			for part in parts {
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					part,
					false,
				)?;
			}
			fb.emit(Instruction::Interpolate(parts.len() as u16), range);
		}
		ExprKind::Fun(FunNode { params, body, .. }) => {
			emit_fun(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				params,
				body,
				range,
			)?;
		}
		ExprKind::Call(CallNode {
			callee,
			args,
			dict_args,
			..
		}) => {
			emit_call(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				callee,
				args,
				dict_args,
				range,
				tail,
			)?;
		}
		ExprKind::FieldAccess { receiver, field } => {
			// Trait method reference: `numeric.add` is a value. Skip the
			// regular field-access lowering (records / enum variants /
			// modules) and emit the dispatch load directly.
			if let Some(cell) = &expr.trait_dispatch {
				emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
			} else {
				emit_field_access(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					receiver,
					field,
					range,
				)?;
			}
		}
		ExprKind::BinaryOperation { op, left, right } => {
			// Trait-dispatched binary operators. Two shapes:
			//   - Arithmetic (`+ - * /`): result is the dispatch's return value.
			//   - Ordering (`< <= > >=`): result is `compare(left, right) {==,!=}
			//     <variant>`, so codegen pushes the matching `ordering`
			//     variant after the `Call(2)` and emits an `Eq`/`Neq`.
			if let Some(cell) = &expr.trait_dispatch {
				emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					left,
					false,
				)?;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					right,
					false,
				)?;
				fb.emit(Instruction::Call(2), range);

				// Tail for ordering ops: push the comparison variant and
				// emit Eq/Neq.
				match &op.kind {
					Operator::LessThan
					| Operator::LessThanEquals
					| Operator::GreaterThan
					| Operator::GreaterThanEquals => {
						let (variant, use_neq) = match op.kind {
							Operator::LessThan => ("lt", false),
							Operator::GreaterThan => ("gt", false),
							Operator::LessThanEquals => ("gt", true),
							Operator::GreaterThanEquals => ("lt", true),
							_ => unreachable!(),
						};
						emit_variant_construction(cg, fb, "__prelude__.ordering", variant, 0, range)?;
						fb.emit(
							if use_neq {
								Instruction::Neq
							} else {
								Instruction::Eq
							},
							range,
						);
					}
					_ => {}
				}
				return Ok(());
			}

			// `x | f a b` lowers to a call `f x a b` — emit callee, then `x`
			// as the first arg, then the rest of the RHS call's args.
			if let Operator::Chain = op.kind {
				let (callee, extra_args) = match &right.kind {
					ExprKind::Call(CallNode { callee, args, .. }) => (callee.as_ref(), args.as_slice()),
					_ => (right.as_ref(), &[][..]),
				};
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					callee,
					false,
				)?;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					left,
					false,
				)?;
				for a in extra_args {
					emit_expr_with_parents(
						cg,
						current_module,
						imports,
						fb,
						scope,
						parent_scopes,
						a,
						false,
					)?;
				}
				let arg_count = (extra_args.len() + 1) as u16;
				let instr = if tail {
					Instruction::TailCall(arg_count)
				} else {
					Instruction::Call(arg_count)
				};
				fb.emit(instr, range);
				return Ok(());
			}

			emit_expr_with_parents(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				left,
				false,
			)?;
			emit_expr_with_parents(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				right,
				false,
			)?;
			let is_float = matches!(left.ty, compiler::types::Type::Float)
				|| matches!(right.ty, compiler::types::Type::Float);
			let instr = match (&op.kind, is_float) {
				(Operator::Addition, false) => Instruction::AddInt,
				(Operator::Addition, true) => Instruction::AddFloat,
				(Operator::SubtractionOrNegation, false) => Instruction::SubInt,
				(Operator::SubtractionOrNegation, true) => Instruction::SubFloat,
				(Operator::Multiplication, false) => Instruction::MulInt,
				(Operator::Multiplication, true) => Instruction::MulFloat,
				(Operator::Division, false) => Instruction::DivInt,
				(Operator::Division, true) => Instruction::DivFloat,
				(Operator::Remainder, false) => Instruction::RemInt,
				(Operator::Remainder, true) => Instruction::RemFloat,
				(Operator::Concat, _) => Instruction::ConcatString,
				(Operator::LogicalAnd, _) => Instruction::LogicalAnd,
				(Operator::LogicalOr, _) => Instruction::LogicalOr,
				(Operator::Equality, _) => Instruction::Eq,
				(Operator::Inequality, _) => Instruction::Neq,
				(Operator::LessThan, _) => Instruction::Lt,
				(Operator::LessThanEquals, _) => Instruction::Lte,
				(Operator::GreaterThan, _) => Instruction::Gt,
				(Operator::GreaterThanEquals, _) => Instruction::Gte,
				_ => {
					return Err(format!("codegen: unhandled binary op {}", op.kind));
				}
			};
			fb.emit(instr, range);
		}
		ExprKind::UnaryOperation { op, right } => {
			// Same trait-dispatch shape as BinaryOp: a unary `-` resolves
			// via `numeric.negate`. Load the dict, pull method 4 (negate),
			// eval operand, Call(1).
			if let Some(cell) = &expr.trait_dispatch {
				emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					right,
					false,
				)?;
				fb.emit(Instruction::Call(1), range);
				return Ok(());
			}
			emit_expr_with_parents(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				right,
				false,
			)?;
			let instr = match op {
				Operator::LogicalNot => Instruction::LogicalNot,
				_ => return Err(format!("codegen: unhandled unary op {}", op)),
			};
			fb.emit(instr, range);
		}
		ExprKind::Regex(node) => {
			let pattern = regex_pattern(node);
			let compiled =
				regex::Regex::new(&pattern).map_err(|e| format!("codegen: invalid regex: {}", e))?;
			let idx = cg.program.regex_patterns.len() as u32;
			cg.program
				.regex_patterns
				.push(Rc::new(RegexData { compiled }));
			fb.emit(Instruction::LoadRegex(idx), range);
		}
		ExprKind::If(IfNode {
			subject,
			pattern,
			body,
			else_body,
			..
		}) => {
			emit_if(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				subject,
				pattern,
				body,
				else_body.as_deref(),
				range,
				tail,
			)?;
		}
		ExprKind::When(WhenNode { subject, cases, .. }) => {
			emit_when(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				subject,
				cases,
				range,
				tail,
			)?;
		}
		ExprKind::While(WhileNode {
			subject,
			pattern,
			body,
			..
		}) => {
			emit_while(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				subject,
				pattern,
				body,
				range,
			)?;
		}
		ExprKind::Scope(node) => {
			emit_scope(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				node,
				range,
				tail,
			)?;
		}
		ExprKind::ElementAccess { .. } => {
			return Err("codegen: ElementAccess not implemented".into());
		}
		ExprKind::Try(TryNode {
			pattern,
			value,
			rest,
			task_carrier,
			..
		}) => {
			// option/result `try`s are rewritten into `<carrier>.then` calls by
			// the analyzer and never reach codegen. A surviving `try` is the
			// task carrier — and, by construction, only ever appears inside an
			// async-bearing function, which `emit_fun` compiles to a step
			// function. We lower it to: evaluate the awaited task, `Await`
			// (suspend), bind the result, then emit the continuation inline —
			// the CPS state-machine transform.
			if !task_carrier {
				return Err("codegen: non-task `try` was not rewritten by the analyzer".into());
			}
			emit_expr_with_parents(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				value,
				false,
			)?;
			fb.emit(Instruction::Await, range);
			match &pattern.kind {
				PatternKind::Identifier(ident) => {
					let slot = fb.alloc_slot();
					fb.emit(Instruction::StoreLocal(slot), range);
					scope.define_local(&ident.name, slot);
				}
				PatternKind::Underscore => {
					fb.emit(Instruction::Pop, range);
				}
				// The analyzer restricts a task `try` pattern to ident/wildcard.
				_ => return Err("codegen: unsupported `try` pattern".into()),
			}
			// The continuation. Its last expr is the function's tail task and
			// inherits this `try`'s tail position.
			for (i, e) in rest.iter().enumerate() {
				let is_last = i == rest.len() - 1;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					e,
					is_last && tail,
				)?;
				if !is_last {
					fb.emit(Instruction::Pop, e.range);
				}
			}
		}
		ExprKind::NamespaceAccess(path) => {
			// Trait method reference (e.g. `numeric.add`): dispatch cell was
			// attached during analysis, same as for a FieldAccess-shaped
			// trait method. Load the dict; skip the path-based lowering.
			if let Some(cell) = &expr.trait_dispatch {
				emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
			} else if let Some(cells) = undrained_dispatch_cells(expr) {
				// Cross-module constrained value (e.g. `mod.step`) used as a
				// first-class value rather than a direct callee. Same fix as
				// the bare-identifier case.
				emit_constrained_value_ref(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					expr,
					&cells,
					range,
				)?;
			} else {
				emit_namespace_access(cg, current_module, imports, fb, path, range)?;
			}
		}
		ExprKind::Builtin(_) => {
			// `built-in` only appears at the immediate RHS of a top-level
			// def, which `compile_module` special-cases (storing the
			// builtin into the global slot directly). Reaching here means
			// the analyzer let one through somewhere it shouldn't have.
			return Err(
				"codegen: `built-in` may only appear as the immediate RHS of a top-level def".into(),
			);
		}
	}
	Ok(())
}

// Codegen for a NamespaceAccess path. The analyzer rewrites three input
// shapes into this node: `module.value` (2 segs, type is the freshened
// imported value type), `EnumName.variant` (2 segs, type is the enum or a
// ctor fn), and `module.EnumName.variant` (3 segs, same).
//
// Trait-method paths (`trait.method`) are also produced by the analyzer but
// are handled before reaching here — they carry a `trait_dispatch` cell on
// the expr and use `emit_dispatch_load` instead.
fn emit_namespace_access(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	path: &[IdentifierNode],
	range: Range,
) -> Result<(), String> {
	match path {
		// `module.EnumName.variant`: cross-module enum variant constructor.
		[module_ident, enum_ident, variant_ident] => {
			let qualified_module = imports.get(&module_ident.name).ok_or_else(|| {
				format!(
					"codegen: namespace `{}` is not an imported module",
					module_ident.name
				)
			})?;
			let qualified_enum = format!("{}.{}", qualified_module, enum_ident.name);
			let variants = cg
				.enum_variants
				.get(&qualified_enum)
				.cloned()
				.ok_or_else(|| format!("codegen: enum `{}` not found", qualified_enum))?;
			let (_, arity) = variants
				.iter()
				.find(|(n, _)| n == &variant_ident.name)
				.ok_or_else(|| {
					format!(
						"codegen: variant `{}` not in `{}`",
						variant_ident.name, qualified_enum
					)
				})?;
			emit_variant_construction(cg, fb, &qualified_enum, &variant_ident.name, *arity, range)
		}
		// 2-segment paths: either `module.value` or `EnumName.variant`.
		// `head` may match both an imported module *and* a local-module
		// enum (e.g. the auto-imported `option` module overlaps with the
		// prelude `option` enum). The analyzer's FieldAccess dispatch
		// resolves the overlap; here we mirror it — module value first,
		// then enum variant.
		[head, tail] => {
			// A dotted head is a compiler-inserted fully-qualified reference
			// (e.g. `??`-over-task lowers to `core.task.or-else`), never a
			// user namespace -- those are bare identifiers. Resolve it as a
			// global directly, independent of the module's imports.
			if head.name.contains('.') {
				if let Some(global_idx) = cg.lookup_global(&head.name, &tail.name) {
					fb.emit(Instruction::LoadGlobal(global_idx), range);
					return Ok(());
				}
				return Err(format!("codegen: `{}.{}` not found", head.name, tail.name));
			}
			if let Some(qualified_module) = imports.get(&head.name).cloned() {
				if let Some(global_idx) = cg.lookup_global(&qualified_module, &tail.name) {
					fb.emit(Instruction::LoadGlobal(global_idx), range);
					return Ok(());
				}
			}
			let qualified_enum = format!("{}.{}", current_module, head.name);
			if let Some(variants) = cg.enum_variants.get(&qualified_enum).cloned() {
				if let Some((_, arity)) = variants.iter().find(|(n, _)| n == &tail.name) {
					return emit_variant_construction(cg, fb, &qualified_enum, &tail.name, *arity, range);
				}
			}
			if imports.get(&head.name).is_some() {
				Err(format!(
					"codegen: `{}.{}` is not defined",
					head.name, tail.name
				))
			} else {
				Err(format!(
					"codegen: namespace `{}` is neither an imported module nor a local enum",
					head.name
				))
			}
		}
		_ => Err(format!(
			"codegen: NamespaceAccess with {} segments — expected 2 or 3",
			path.len()
		)),
	}
}

fn emit_literal_with_cg(
	cg: &mut CodeGen,
	fb: &mut FunctionBuilder,
	kind: &LiteralKind,
	range: Range,
) {
	match kind {
		LiteralKind::Bool(b) => {
			fb.emit(Instruction::LoadBool(*b), range);
		}
		LiteralKind::String(s) => {
			let idx = cg.intern(s);
			fb.emit(Instruction::LoadConst(idx), range);
		}
		LiteralKind::Bytes(b) => {
			let idx = cg.intern_bytes(b);
			fb.emit(Instruction::LoadBytes(idx), range);
		}
		LiteralKind::FloatDecimal(f) => {
			fb.emit(Instruction::LoadFloat(*f), range);
		}
		LiteralKind::Duration(n) => {
			fb.emit(Instruction::LoadDuration(*n), range);
		}
		LiteralKind::IntDecimal(n)
		| LiteralKind::IntHex(n)
		| LiteralKind::IntOctal(n)
		| LiteralKind::IntBinary(n) => {
			fb.emit(Instruction::LoadInt(*n as i64), range);
		}
	}
}

// ------- helper functions used above -------

fn emit_identifier(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	ident: &IdentifierNode,
	range: Range,
) -> Result<(), String> {
	let mut parent_refs: Vec<&mut Scope> =
		parent_scopes.iter().map(|p| unsafe { &mut **p }).collect();
	let res = resolve_identifier(
		cg,
		current_module,
		imports,
		scope,
		parent_refs.as_mut_slice(),
		&ident.name,
	)
	.ok_or_else(|| format!("codegen: unbound identifier `{}`", ident.name))?;
	match res {
		Resolution::Local(slot) => {
			fb.emit(Instruction::LoadLocal(slot), range);
		}
		Resolution::Capture(idx) => {
			fb.emit(Instruction::LoadCapture(idx), range);
		}
		Resolution::Global(idx) => {
			fb.emit(Instruction::LoadGlobal(idx), range);
		}
		Resolution::BareVariant(qualified_enum, variant_name, arity) => {
			emit_variant_construction(cg, fb, &qualified_enum, &variant_name, arity, range)?;
		}
		Resolution::EnumName(_) | Resolution::Imported(_) => {
			return Err(format!(
				"codegen: `{}` is a namespace, not a value",
				ident.name
			));
		}
	}
	Ok(())
}

// If `expr` carries a non-empty, undrained dispatch sink, return its cells.
// A Call drains its callee's sink into the Call's `dict_args` during
// analysis (`annotate_expr`), so a sink that survives to codegen means the
// reference is in *value* position — passed as an argument, returned, or
// bound — rather than directly applied. Those are exactly the references
// that need their dictionaries pre-applied (`emit_constrained_value_ref`).
// An empty sink (an unconstrained def referenced as a value) is treated as
// absent so it lowers as a plain reference.
fn undrained_dispatch_cells(expr: &ExprNode) -> Option<Vec<compiler::ast::DispatchCell>> {
	let sink = expr.dispatch_sink.as_ref()?;
	let cells = sink.borrow();
	if cells.is_empty() {
		None
	} else {
		Some(cells.iter().cloned().collect())
	}
}

// Emit a constrained function referenced as a first-class value.
//
// A top-level def that uses a trait method over a still-polymorphic
// parameter compiles to a function with K hidden leading dict params
// (slots 0..K-1) before its N user params (`compile_constrained_thunk`).
// At a *call* the surrounding Call supplies those dicts as `dict_args`, so
// the arity lines up. But when the def is referenced as a value (passed to
// `list.fold`, stored, returned), nothing supplies the dicts, and the later
// call — which only knows the user-visible arity N — calls it with N args
// against a K+N-arity function, tripping the VM's arity check.
//
// Fix: emit a wrapper closure of arity N that captures the K resolved dicts
// and forwards to the underlying function with the dicts prepended. The
// resulting value's runtime arity matches its user-visible type, so it can
// flow through any higher-order function unchanged.
fn emit_constrained_value_ref(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	expr: &ExprNode,
	cells: &[compiler::ast::DispatchCell],
	range: Range,
) -> Result<(), String> {
	let k = cells.len() as u16;
	// User-visible arity comes from the reference's function type. Trait
	// constraints only quantify functions, so this is always a `Fun`.
	let n = match &expr.ty {
		compiler::types::Type::Fun(params, _) => params.len() as u16,
		other => {
			return Err(format!(
				"codegen: constrained value reference has non-function type `{}`",
				other
			))
		}
	};
	// The underlying K+N-arity function. Both reference shapes that carry a
	// dispatch sink (bare identifier, cross-module `module.value`) resolve to
	// a global slot.
	let global_idx =
		resolve_constrained_ref_global(cg, current_module, imports, scope, parent_scopes, expr)?;

	// Wrapper: arity N, K captures (the dicts). Body re-pushes the underlying
	// function, the captured dicts, then its own params, and tail-calls with
	// the full K+N arity.
	let mut wrapper = FunctionBuilder::new(
		format!("partial@{}", global_idx),
		current_module.to_string(),
		n,
	);
	wrapper.emit(Instruction::LoadGlobal(global_idx), range);
	for i in 0..k {
		wrapper.emit(Instruction::LoadCapture(i), range);
	}
	for i in 0..n {
		wrapper.emit(Instruction::LoadLocal(i), range);
	}
	wrapper.emit(Instruction::TailCall(k + n), range);
	wrapper.emit(Instruction::Return, range);
	wrapper.capture_count = k;
	let wrapper_idx = cg.add_function(wrapper);

	// Outer site: load each resolved dict (using the surrounding scope, so
	// `Forwarded` dicts capture the enclosing def's dict params), then build
	// the wrapper closure capturing them.
	for cell in cells {
		emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
	}
	fb.emit(
		Instruction::MakeClosure {
			fn_idx: wrapper_idx,
			num_captures: k,
		},
		range,
	);
	Ok(())
}

// Resolve the global slot of a constrained value reference (a bare
// identifier or an imported `module.value` namespace access). Used by
// `emit_constrained_value_ref` to bake the load into the wrapper.
fn resolve_constrained_ref_global(
	cg: &CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	expr: &ExprNode,
) -> Result<GlobalIdx, String> {
	match &expr.kind {
		ExprKind::Identifier(ident) => {
			let mut parent_refs: Vec<&mut Scope> =
				parent_scopes.iter().map(|p| unsafe { &mut **p }).collect();
			match resolve_identifier(
				cg,
				current_module,
				imports,
				scope,
				parent_refs.as_mut_slice(),
				&ident.name,
			) {
				Some(Resolution::Global(idx)) => Ok(idx),
				_ => Err(format!(
					"codegen: constrained value `{}` did not resolve to a global",
					ident.name
				)),
			}
		}
		ExprKind::NamespaceAccess(path) => match path.as_slice() {
			[head, tail] => {
				let qualified_module = imports.get(&head.name).ok_or_else(|| {
					format!(
						"codegen: namespace `{}` is not an imported module",
						head.name
					)
				})?;
				cg.lookup_global(qualified_module, &tail.name)
					.ok_or_else(|| {
						format!(
							"codegen: constrained value `{}.{}` is not a global",
							head.name, tail.name
						)
					})
			}
			_ => Err(format!(
				"codegen: constrained value reference has a {}-segment namespace path",
				path.len()
			)),
		},
		_ => Err("codegen: constrained value reference is neither identifier nor namespace".into()),
	}
}

fn emit_fun(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	params: &[compiler::ast::FunParamNode],
	body: &[ExprNode],
	range: Range,
) -> Result<(), String> {
	// Compile the inner function's body in a fresh scope, with the current
	// scope visible as the parent.
	let mut inner_scope = Scope::new();
	for (i, p) in params.iter().enumerate() {
		inner_scope.define_local(&p.ident.name, i as u16);
	}
	let mut inner_fb = FunctionBuilder::new(
		format!("fun@{}:{}", range.start.line, range.start.col),
		current_module.to_string(),
		params.len() as u16,
	);

	// Set up parent_scopes for the inner emission: enclose current scope.
	parent_scopes.push(scope as *mut Scope);
	let res = (|| -> Result<(), String> {
		if body.is_empty() {
			inner_fb.emit(Instruction::LoadNothing, range);
		} else {
			for (i, e) in body.iter().enumerate() {
				let is_last = i == body.len() - 1;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					&mut inner_fb,
					&mut inner_scope,
					parent_scopes,
					e,
					is_last,
				)?;
				if !is_last {
					inner_fb.emit(Instruction::Pop, e.range);
				}
			}
		}
		inner_fb.emit(Instruction::Return, range);
		Ok(())
	})();
	parent_scopes.pop();
	res?;

	// `inner_fb.capture_count` and `inner_scope.captures` describe the
	// captures the inner function needs. Push them onto the operand stack
	// in order, then MakeClosure.
	let captures = std::mem::take(&mut inner_scope.captures);
	inner_fb.capture_count = captures.len() as u16;
	let inner_fn_idx = cg.add_function(inner_fb);

	for cap in &captures {
		match &cap.source {
			CaptureSource::Local(slot) => {
				fb.emit(Instruction::LoadLocal(*slot), range);
			}
			CaptureSource::Capture(idx) => {
				fb.emit(Instruction::LoadCapture(*idx), range);
			}
		}
	}
	// An async-bearing function (its body awaits a task via `try`) becomes a
	// `Value::AsyncFn`: calling it builds a cold task instead of running. The
	// emitted body bytecode is identical either way — only the `Await`
	// suspension points (lowered in the `Try` arm) and this wrapper differ.
	let num_captures = captures.len() as u16;
	let instr = if body_is_async(body) {
		Instruction::MakeAsyncClosure {
			fn_idx: inner_fn_idx,
			num_captures,
		}
	} else {
		Instruction::MakeClosure {
			fn_idx: inner_fn_idx,
			num_captures,
		}
	};
	fb.emit(instr, range);
	Ok(())
}

// Lower a `scope` block to `task.scope-new <manual> (fun handle { body })`:
// push the kernel builtin, the manual flag, and the body-as-closure, then call.
// The body becomes its own closure frame (so its `try`s suspend within the
// scope's child fiber, not this one); the runtime driver creates the scope and
// runs that closure when the resulting task is awaited. See `vm::task`.
fn emit_scope(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	node: &ScopeNode,
	range: Range,
	tail: bool,
) -> Result<(), String> {
	let g = cg
		.lookup_global("core.task", "scope-new")
		.ok_or("codegen: `core.task.scope-new` not found")?;
	fb.emit(Instruction::LoadGlobal(g), range);
	fb.emit(Instruction::LoadBool(node.manual), range);

	// The body closure's parameter carries the `scope as NAME` handle name so
	// the body's `s.*` references resolve to it; an anonymous scope gets an
	// unreferenced synthetic parameter.
	let handle_ident = node.handle.clone().unwrap_or_else(|| IdentifierNode {
		name: "__scope".to_string(),
		range,
	});
	let params = [FunParamNode {
		ident: handle_ident,
		ty: Type::Nothing,
	}];
	emit_fun(
		cg,
		current_module,
		imports,
		fb,
		scope,
		parent_scopes,
		&params,
		&node.body,
		range,
	)?;

	let instr = if tail {
		Instruction::TailCall(2)
	} else {
		Instruction::Call(2)
	};
	fb.emit(instr, range);
	Ok(())
}

// Is this function body async-bearing — i.e. does it directly await a task?
// True iff it contains a task-carrier `try` in its *own* frame. Crucially we
// do NOT descend into nested `Fun`s: those are separate functions whose own
// async-ness is decided when they're emitted. Must stay exhaustive over the
// same expression forms a `try` can hide inside (control flow, let, defer,
// groupings) so a step function is never miscompiled as a plain closure.
fn body_is_async(body: &[ExprNode]) -> bool {
	body.iter().any(expr_is_async)
}

fn expr_is_async(expr: &ExprNode) -> bool {
	match &expr.kind {
		ExprKind::Try(TryNode {
			task_carrier,
			value,
			rest,
			..
		}) => {
			// A task `try` makes this frame async. (Even a non-task `try` here
			// would be a bug, but recurse defensively into its sub-trees.)
			*task_carrier || expr_is_async(value) || rest.iter().any(expr_is_async)
		}
		// Stop at function boundaries — a nested closure is its own frame.
		ExprKind::Fun(_) => false,
		ExprKind::Let(LetNode { value, .. }) => expr_is_async(value),
		ExprKind::Defer(inner) | ExprKind::Grouping(inner) => expr_is_async(inner),
		ExprKind::Call(CallNode { callee, args, .. }) => {
			expr_is_async(callee) || args.iter().any(expr_is_async)
		}
		ExprKind::Tuple(es) | ExprKind::Interpolation(es) => es.iter().any(expr_is_async),
		ExprKind::List(items) => items.iter().any(|it| expr_is_async(it.expr())),
		ExprKind::Record(fields) => fields.iter().any(|(_, v)| expr_is_async(v)),
		ExprKind::ElementAccess { receiver, .. } | ExprKind::FieldAccess { receiver, .. } => {
			expr_is_async(receiver)
		}
		ExprKind::UnaryOperation { right, .. } => expr_is_async(right),
		ExprKind::BinaryOperation { left, right, .. } => expr_is_async(left) || expr_is_async(right),
		ExprKind::If(IfNode {
			subject,
			body,
			else_body,
			..
		}) => {
			expr_is_async(subject)
				|| body.iter().any(expr_is_async)
				|| else_body
					.as_ref()
					.map_or(false, |b| b.iter().any(expr_is_async))
		}
		ExprKind::When(WhenNode { subject, cases, .. }) => {
			expr_is_async(subject) || cases.iter().any(|c| c.body.iter().any(expr_is_async))
		}
		ExprKind::While(WhileNode { subject, body, .. }) => {
			expr_is_async(subject) || body.iter().any(expr_is_async)
		}
		// A `scope` block's body becomes its own closure frame (lowered in
		// `emit_scope`), so its internal `try`s don't make *this* frame async —
		// like a nested `Fun`. The scope expression itself just builds a task.
		ExprKind::Scope(_) => false,
		ExprKind::Identifier(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::EmptyTuple
		| ExprKind::Builtin(_)
		| ExprKind::NamespaceAccess(_) => false,
	}
}

fn emit_call(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	callee: &ExprNode,
	args: &[ExprNode],
	dict_args: &[compiler::ast::DispatchCell],
	range: Range,
	tail: bool,
) -> Result<(), String> {
	emit_expr_with_parents(
		cg,
		current_module,
		imports,
		fb,
		scope,
		parent_scopes,
		callee,
		false,
	)?;
	// Hidden dict args are emitted between callee and user args. The
	// callee's compiled function expects them at slot 0..K-1 (K =
	// dict_args.len()) and the user args at slot K..K+arity-1.
	for cell in dict_args {
		emit_dispatch_load(cg, fb, scope, parent_scopes, cell, range)?;
	}
	for a in args {
		emit_expr_with_parents(
			cg,
			current_module,
			imports,
			fb,
			scope,
			parent_scopes,
			a,
			false,
		)?;
	}
	let total_arity = (dict_args.len() + args.len()) as u16;
	let instr = if tail {
		Instruction::TailCall(total_arity)
	} else {
		Instruction::Call(total_arity)
	};
	fb.emit(instr, range);
	Ok(())
}

// Emit instructions to load a dispatch dictionary onto the stack,
// resolving according to the cell's `Resolved` value. `Global(name)`
// reads from the named prelude/instance global; `Forwarded(slot)`
// reads the synthetic dict local `__dict_<slot>__`, which the
// constrained-def thunk defines at slot `slot`. Using the same scope
// resolution path as named identifiers means inner closures
// automatically capture the dict — no special closure path needed.
fn emit_dispatch_load(
	cg: &mut CodeGen,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	cell: &compiler::ast::DispatchCell,
	range: Range,
) -> Result<(), String> {
	use compiler::ast::Resolved;
	let borrow = cell.borrow();
	match &borrow.resolved {
		Some(Resolved::Global(slot_name)) => {
			let (module, name) = match slot_name.rsplit_once('.') {
				Some((m, n)) => (m, n),
				None => {
					return Err(format!(
						"codegen: malformed instance slot name `{}`",
						slot_name
					))
				}
			};
			let global_idx = cg.lookup_global(module, name).ok_or_else(|| {
				format!(
					"codegen: instance slot `{}` not registered as a global",
					slot_name
				)
			})?;
			fb.emit(Instruction::LoadGlobal(global_idx), range);
		}
		Some(Resolved::Forwarded(slot)) => {
			// Look up the synthetic dict name in the scope chain. The
			// resolver handles the cross-Fun case by adding a closure
			// capture — so a Forwarded dispatch in a nested lambda
			// automatically captures the outer fn's dict.
			let name = synthetic_dict_name(*slot);
			let mut parent_refs: Vec<&mut Scope> =
				parent_scopes.iter().map(|p| unsafe { &mut **p }).collect();
			let res = resolve_identifier(
				cg,
				"",
				&HashMap::new(),
				scope,
				parent_refs.as_mut_slice(),
				&name,
			)
			.ok_or_else(|| {
				format!(
					"codegen: dispatch slot `{}` not found in any enclosing constrained def",
					name
				)
			})?;
			match res {
				Resolution::Local(s) => {
					fb.emit(Instruction::LoadLocal(s), range);
				}
				Resolution::Capture(idx) => {
					fb.emit(Instruction::LoadCapture(idx), range);
				}
				_ => {
					return Err(format!(
						"codegen: dispatch slot `{}` resolved to an unexpected source",
						name
					));
				}
			}
		}
		Some(Resolved::InstanceChain { ctor_slot, inner }) => {
			let (module, name) = match ctor_slot.rsplit_once('.') {
				Some((m, n)) => (m, n),
				None => return Err(format!("codegen: malformed ctor slot name `{}`", ctor_slot)),
			};
			let global_idx = cg.lookup_global(module, name).ok_or_else(|| {
				format!(
					"codegen: ctor slot `{}` not registered as a global",
					ctor_slot
				)
			})?;
			// Load the constructor (a closure), push each inner dict,
			// then call to materialize the parametric dict. We borrow
			// `cell` for the resolve; cloning what we need above lets us
			// drop the borrow before recursing on inner cells, each of
			// which `borrow_mut`s its own cell.
			let inner_cloned: Vec<Resolved> = inner.clone();
			drop(borrow);
			fb.emit(Instruction::LoadGlobal(global_idx), range);
			for r in &inner_cloned {
				emit_resolved_load(cg, fb, scope, parent_scopes, r, range)?;
			}
			fb.emit(Instruction::Call(inner_cloned.len() as u16), range);
			// Re-borrow for the optional GetDictField pass-through.
			let borrow = cell.borrow();
			if let Some(idx) = borrow.method_idx {
				fb.emit(Instruction::GetDictField(idx as u16), range);
			}
			return Ok(());
		}
		None => {
			return Err(format!(
				"codegen: dispatch cell for trait `{}` is unresolved",
				borrow.trait_name
			));
		}
	}
	// Extract the specific method from the dict, if this is a method-
	// dispatch site (`method_idx = Some`). Call-forwarding sites
	// (`method_idx = None`) push the whole dict, and the callee's
	// codegen does the method extraction itself via its own cells.
	if let Some(idx) = borrow.method_idx {
		fb.emit(Instruction::GetDictField(idx as u16), range);
	}
	Ok(())
}

fn synthetic_dict_name(slot: u16) -> String {
	format!("__dict_{}__", slot)
}

// Emit the load for a `Resolved` value that came from a parametric
// `InstanceChain`'s inner list — no method extraction, just push the
// dict onto the stack.
fn emit_resolved_load(
	cg: &mut CodeGen,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	r: &compiler::ast::Resolved,
	range: Range,
) -> Result<(), String> {
	use compiler::ast::Resolved;
	match r {
		Resolved::Global(slot_name) => {
			let (module, name) = match slot_name.rsplit_once('.') {
				Some((m, n)) => (m, n),
				None => return Err(format!("codegen: malformed slot name `{}`", slot_name)),
			};
			let global_idx = cg
				.lookup_global(module, name)
				.ok_or_else(|| format!("codegen: slot `{}` not registered as a global", slot_name))?;
			fb.emit(Instruction::LoadGlobal(global_idx), range);
		}
		Resolved::Forwarded(slot) => {
			let name = synthetic_dict_name(*slot);
			let mut parent_refs: Vec<&mut Scope> =
				parent_scopes.iter().map(|p| unsafe { &mut **p }).collect();
			let res = resolve_identifier(
				cg,
				"",
				&HashMap::new(),
				scope,
				parent_refs.as_mut_slice(),
				&name,
			)
			.ok_or_else(|| format!("codegen: dispatch slot `{}` not found", name))?;
			match res {
				Resolution::Local(s) => {
					fb.emit(Instruction::LoadLocal(s), range);
				}
				Resolution::Capture(idx) => {
					fb.emit(Instruction::LoadCapture(idx), range);
				}
				_ => {
					return Err(format!(
						"codegen: dispatch slot `{}` resolved to an unexpected source",
						name
					));
				}
			}
		}
		Resolved::InstanceChain { ctor_slot, inner } => {
			let (module, name) = match ctor_slot.rsplit_once('.') {
				Some((m, n)) => (m, n),
				None => return Err(format!("codegen: malformed ctor slot name `{}`", ctor_slot)),
			};
			let global_idx = cg.lookup_global(module, name).ok_or_else(|| {
				format!(
					"codegen: ctor slot `{}` not registered as a global",
					ctor_slot
				)
			})?;
			fb.emit(Instruction::LoadGlobal(global_idx), range);
			for r in inner {
				emit_resolved_load(cg, fb, scope, parent_scopes, r, range)?;
			}
			fb.emit(Instruction::Call(inner.len() as u16), range);
		}
	}
	Ok(())
}

fn emit_field_access(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	receiver: &ExprNode,
	field: &IdentifierNode,
	range: Range,
) -> Result<(), String> {
	// 1. Chained `module.enum.variant`?
	if let ExprKind::FieldAccess {
		receiver: outer,
		field: enum_field,
	} = &receiver.kind
	{
		if let ExprKind::Identifier(module_ident) = &outer.kind {
			if let Some(qualified_module) = imports.get(&module_ident.name) {
				let qualified_enum = format!("{}.{}", qualified_module, enum_field.name);
				if let Some(variants) = cg.enum_variants.get(&qualified_enum).cloned() {
					if let Some((_, arity)) = variants.iter().find(|(n, _)| n == &field.name) {
						return emit_variant_construction(cg, fb, &qualified_enum, &field.name, *arity, range);
					}
				}
			}
		}
	}

	// 2. `module.value` import access?
	if let ExprKind::Identifier(ident) = &receiver.kind {
		if let Some(qualified_module) = imports.get(&ident.name).cloned() {
			if let Some(global_idx) = cg.lookup_global(&qualified_module, &field.name) {
				fb.emit(Instruction::LoadGlobal(global_idx), range);
				return Ok(());
			}
			return Err(format!(
				"codegen: `{}.{}` is not defined",
				ident.name, field.name
			));
		}
	}

	// 3. Local `enum-name.variant`?
	if let ExprKind::Identifier(ident) = &receiver.kind {
		let qualified_enum = format!("{}.{}", current_module, ident.name);
		if let Some(variants) = cg.enum_variants.get(&qualified_enum).cloned() {
			if let Some((_, arity)) = variants.iter().find(|(n, _)| n == &field.name) {
				return emit_variant_construction(cg, fb, &qualified_enum, &field.name, *arity, range);
			}
		}
	}

	// 4. Record field access.
	emit_expr_with_parents(
		cg,
		current_module,
		imports,
		fb,
		scope,
		parent_scopes,
		receiver,
		false,
	)?;
	let name_idx = cg.intern(&field.name);
	fb.emit(Instruction::GetField(name_idx), range);
	Ok(())
}

fn emit_variant_construction(
	cg: &mut CodeGen,
	fb: &mut FunctionBuilder,
	qualified_enum: &str,
	variant_name: &str,
	arity: usize,
	range: Range,
) -> Result<(), String> {
	let q_idx = cg.intern(qualified_enum);
	let v_idx = cg.intern(variant_name);
	if arity == 0 {
		fb.emit(
			Instruction::MakeVariant {
				qualified: q_idx,
				variant: v_idx,
				arity: 0,
			},
			range,
		);
	} else {
		fb.emit(
			Instruction::MakeVariantCtor {
				qualified: q_idx,
				variant: v_idx,
				arity: arity as u16,
			},
			range,
		);
	}
	Ok(())
}

fn emit_if(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	subject: &ExprNode,
	pattern: &PatternNode,
	body: &[ExprNode],
	else_body: Option<&[ExprNode]>,
	range: Range,
	tail: bool,
) -> Result<(), String> {
	// `if X is P { body }` — match, run body, else skip. Always evaluates
	// to nothing if there's no else — so the body's expressions are never
	// in tail position (their values get popped). With `else`, the if is
	// a value expression: each branch's last expression stays on the
	// stack and the if takes that type.
	emit_expr_with_parents(
		cg,
		current_module,
		imports,
		fb,
		scope,
		parent_scopes,
		subject,
		false,
	)?;
	let subject_ty = subject.ty.clone();
	scope.enter();
	let fail_idx = emit_pattern(cg, fb, scope, &subject_ty, pattern)?;
	let has_else = else_body.is_some();
	if body.is_empty() && has_else {
		fb.emit(Instruction::LoadNothing, range);
	} else {
		for (i, e) in body.iter().enumerate() {
			let is_last = i == body.len() - 1;
			emit_expr_with_parents(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				e,
				is_last && has_else && tail,
			)?;
			// Without `else`, every body value is discarded. With `else`,
			// the last value is the if's result and stays on the stack.
			if !(is_last && has_else) {
				fb.emit(Instruction::Pop, e.range);
			}
		}
	}
	let end_jump = fb.emit(Instruction::Jump(0), range);
	let fail_target = fb.here();
	for fi in fail_idx {
		fb.patch_jump(fi, fail_target);
	}
	match else_body {
		Some(else_body) => {
			if else_body.is_empty() {
				fb.emit(Instruction::LoadNothing, range);
			} else {
				for (i, e) in else_body.iter().enumerate() {
					let is_last = i == else_body.len() - 1;
					emit_expr_with_parents(
						cg,
						current_module,
						imports,
						fb,
						scope,
						parent_scopes,
						e,
						is_last && tail,
					)?;
					if !is_last {
						fb.emit(Instruction::Pop, e.range);
					}
				}
			}
		}
		None => {}
	}
	let end = fb.here();
	fb.patch_jump(end_jump, end);
	scope.leave();
	if !has_else {
		fb.emit(Instruction::LoadNothing, range);
	}
	Ok(())
}

fn emit_when(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	subject: &ExprNode,
	cases: &[compiler::ast::CaseNode],
	range: Range,
	tail: bool,
) -> Result<(), String> {
	emit_expr_with_parents(
		cg,
		current_module,
		imports,
		fb,
		scope,
		parent_scopes,
		subject,
		false,
	)?;
	let subject_ty = subject.ty.clone();
	// For each case: dup the subject, attempt match, if fail jump to next.
	// On success, evaluate body, push its value, then jump to end. After
	// all cases, if none matched we're in trouble — but the analyzer
	// enforces exhaustiveness for known finite types. Emit a runtime trap.
	let mut end_jumps = Vec::new();
	for (i, case) in cases.iter().enumerate() {
		let is_last = i == cases.len() - 1;
		// Dup the subject for this attempt (so the next case can also try).
		// On the last case we don't need to dup because no more attempts.
		if !is_last {
			fb.emit(Instruction::Dup, case.range);
		}
		scope.enter();
		let fail_indices = emit_pattern(cg, fb, scope, &subject_ty, &case.pattern)?;
		// Match succeeded; the dup'd subject was consumed by emit_pattern
		// (each match-instruction pops its subject). If this case is the
		// last one, the original subject was the one consumed; otherwise we
		// dup'd, so the original is still beneath us.
		if !is_last {
			// We dup'd, so an extra copy of the subject is still on the
			// stack BELOW where we now are. We need to remove it after a
			// successful match. The success path runs:
			//   - emit_pattern consumed the dup
			//   - now stack has: [orig_subject, ...payload_bindings_stored_in_locals...]
			// So we need to pop the original from underneath. Easiest:
			// since pattern emission already stored bindings in locals,
			// the operand stack is back to just [orig_subject]. Pop it.
			fb.emit(Instruction::Pop, case.range);
		}
		// Evaluate body expressions; last one is the case's result, which
		// is also the when's result — so it's in tail position iff the
		// when itself is.
		if case.body.is_empty() {
			fb.emit(Instruction::LoadNothing, case.range);
		} else {
			for (i, e) in case.body.iter().enumerate() {
				let is_last = i == case.body.len() - 1;
				emit_expr_with_parents(
					cg,
					current_module,
					imports,
					fb,
					scope,
					parent_scopes,
					e,
					is_last && tail,
				)?;
				if !is_last {
					fb.emit(Instruction::Pop, e.range);
				}
			}
		}
		scope.leave();
		end_jumps.push(fb.emit(Instruction::Jump(0), case.range));
		// Patch the fail jumps to land at the next case's start.
		let next_case_start = fb.here();
		for fi in fail_indices {
			fb.patch_jump(fi, next_case_start);
		}
		// On failure path, the subject was consumed by the failing match
		// instruction. If we dup'd earlier, the original is still on the
		// stack; if we didn't (last case), there's nothing left. Either
		// way, control reaches here only on no-match — and if this is the
		// last case we should trap. (The analyzer's exhaustiveness check
		// should prevent this, but as a safety net we emit instructions
		// that push Nothing and jump to end. The when expression's result
		// type may not be Nothing, so this is a known fudge — see
		// PERF-NOTES.)
	}
	// All cases failed (only reachable when exhaustiveness checking would
	// have caught a real bug elsewhere; emit a Nothing for safety).
	fb.emit(Instruction::LoadNothing, range);
	let end = fb.here();
	for j in end_jumps {
		fb.patch_jump(j, end);
	}
	Ok(())
}

fn emit_while(
	cg: &mut CodeGen,
	current_module: &str,
	imports: &HashMap<String, String>,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	parent_scopes: &mut Vec<*mut Scope>,
	subject: &ExprNode,
	pattern: &PatternNode,
	body: &[ExprNode],
	range: Range,
) -> Result<(), String> {
	// loop_top:
	//   eval subject  (re-evaluates each iteration; subject may have side effects)
	//   match pattern -> on fail, jump to exit
	//   eval body (popping each result)
	//   jump loop_top
	// exit:
	//   push Nothing
	let loop_top = fb.here();
	emit_expr_with_parents(
		cg,
		current_module,
		imports,
		fb,
		scope,
		parent_scopes,
		subject,
		false,
	)?;
	let subject_ty = subject.ty.clone();
	scope.enter();
	let fail_idx = emit_pattern(cg, fb, scope, &subject_ty, pattern)?;
	for e in body {
		emit_expr_with_parents(
			cg,
			current_module,
			imports,
			fb,
			scope,
			parent_scopes,
			e,
			false,
		)?;
		fb.emit(Instruction::Pop, e.range);
	}
	fb.emit(Instruction::Jump(loop_top), range);
	let exit = fb.here();
	for fi in fail_idx {
		fb.patch_jump(fi, exit);
	}
	scope.leave();
	fb.emit(Instruction::LoadNothing, range);
	Ok(())
}

// --------------------------------------------------------------------------
// Pattern emission.
// --------------------------------------------------------------------------

// Emits instructions to attempt to match the subject on top of the stack
// against `pattern`. On success: bindings are stored in locals, the
// subject's pieces are consumed from the stack. On failure: the subject
// is consumed and execution jumps to one of the returned offsets.
//
// Returns the list of jump instruction indices (with placeholder 0
// targets) that the caller must patch to point to its no-match handler.
fn emit_pattern(
	cg: &mut CodeGen,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	subject_ty: &compiler::types::Type,
	pattern: &PatternNode,
) -> Result<Vec<u32>, String> {
	let range = pattern.range;
	let mut fails = Vec::new();
	match &pattern.kind {
		PatternKind::Underscore => {
			fb.emit(Instruction::Pop, range);
		}
		PatternKind::Identifier(ident) => {
			// Disambiguate against nullary variant of the subject's enum.
			let is_variant_match = if let compiler::types::Type::Enum(qualified, _) = subject_ty {
				cg.enum_variants
					.get(qualified)
					.map(|vs| vs.iter().any(|(n, arity)| n == &ident.name && *arity == 0))
					.unwrap_or(false)
			} else {
				false
			};
			if is_variant_match {
				let v_idx = cg.intern(&ident.name);
				let jmp = fb.emit(
					Instruction::MatchVariant {
						variant: v_idx,
						arity: 0,
						on_fail: 0,
					},
					range,
				);
				fails.push(jmp);
			} else if ident.name == "true" && matches!(subject_ty, compiler::types::Type::Bool) {
				let jmp = fb.emit(Instruction::MatchBool(true, 0), range);
				fails.push(jmp);
			} else if ident.name == "false" && matches!(subject_ty, compiler::types::Type::Bool) {
				let jmp = fb.emit(Instruction::MatchBool(false, 0), range);
				fails.push(jmp);
			} else {
				// Identifier binding: pop subject, store in fresh slot.
				let slot = fb.alloc_slot();
				fb.emit(Instruction::StoreLocal(slot), range);
				scope.define_local(&ident.name, slot);
			}
		}
		PatternKind::Literal(lit) => {
			let jmp = match &lit.kind {
				LiteralKind::Bool(b) => fb.emit(Instruction::MatchBool(*b, 0), range),
				LiteralKind::String(s) => {
					let idx = cg.intern(s);
					fb.emit(Instruction::MatchString(idx, 0), range)
				}
				LiteralKind::Bytes(b) => {
					let idx = cg.intern_bytes(b);
					fb.emit(Instruction::MatchBytes(idx, 0), range)
				}
				LiteralKind::FloatDecimal(f) => fb.emit(Instruction::MatchFloat(*f, 0), range),
				LiteralKind::Duration(n) => fb.emit(Instruction::MatchDuration(*n, 0), range),
				LiteralKind::IntDecimal(n)
				| LiteralKind::IntHex(n)
				| LiteralKind::IntOctal(n)
				| LiteralKind::IntBinary(n) => fb.emit(Instruction::MatchInt(*n as i64, 0), range),
			};
			fails.push(jmp);
		}
		PatternKind::Tuple(elems) => {
			let jmp = fb.emit(
				Instruction::MatchTuple {
					arity: elems.len() as u16,
					on_fail: 0,
				},
				range,
			);
			fails.push(jmp);
			// Tuple elements were pushed onto the stack last-on-top.
			// Sub-patterns match in source order, which corresponds to
			// reverse stack order.
			emit_sub_patterns_with_cleanup(
				cg,
				fb,
				scope,
				elems.iter().rev(),
				elems.len(),
				range,
				&mut fails,
			)?;
		}
		PatternKind::Record { fields, rest } => {
			let field_idxs: Vec<u32> = fields.iter().map(|(n, _)| cg.intern(&n.name)).collect();
			let fields_idx = cg.intern_field_list(field_idxs);
			let exact = rest.is_none();
			let with_rest = matches!(
				rest,
				Some(rp) if rp.binding.is_some()
			);
			let jmp = fb.emit(
				Instruction::MatchRecord {
					fields_idx,
					exact,
					with_rest,
					on_fail: 0,
				},
				range,
			);
			fails.push(jmp);
			// When `with_rest` is true, the rest record sits on top of the
			// named field values. Consume it first (bind the named rest
			// ident), then process the named fields in reverse.
			if with_rest {
				let rp = rest.as_ref().unwrap();
				let ident = rp.binding.as_ref().unwrap();
				let slot = fb.alloc_slot();
				fb.emit(Instruction::StoreLocal(slot), rp.range);
				scope.define_local(&ident.name, slot);
			}
			let n = fields.len();
			emit_sub_patterns_with_cleanup(
				cg,
				fb,
				scope,
				fields.iter().rev().map(|(_, p)| p),
				n,
				range,
				&mut fails,
			)?;
		}
		PatternKind::Constructor(variant_name, sub_patterns) => {
			let v_idx = cg.intern(&variant_name.name);
			let jmp = fb.emit(
				Instruction::MatchVariant {
					variant: v_idx,
					arity: sub_patterns.len() as u16,
					on_fail: 0,
				},
				range,
			);
			fails.push(jmp);
			emit_sub_patterns_with_cleanup(
				cg,
				fb,
				scope,
				sub_patterns.iter().rev(),
				sub_patterns.len(),
				range,
				&mut fails,
			)?;
		}
		PatternKind::List { items, rest } => {
			let arity = items.len() as u16;
			let has_rest = rest.is_some();
			let jmp = fb.emit(
				Instruction::MatchList {
					arity,
					has_rest,
					on_fail: 0,
				},
				range,
			);
			fails.push(jmp);
			// Stack discipline on success: items[0..arity] in order (last
			// item on top), and if has_rest, the tail list on top of that.
			// Sub-patterns are consumed in reverse stack order, so handle
			// rest first (if present), then items in reverse.
			if let Some(rp) = rest {
				match &rp.binding {
					Some(ident) => {
						let slot = fb.alloc_slot();
						fb.emit(Instruction::StoreLocal(slot), rp.range);
						scope.define_local(&ident.name, slot);
					}
					None => {
						fb.emit(Instruction::Pop, rp.range);
					}
				}
			}
			emit_sub_patterns_with_cleanup(
				cg,
				fb,
				scope,
				items.iter().rev(),
				items.len(),
				range,
				&mut fails,
			)?;
		}

		PatternKind::Interpolation(_) => {
			return Err("codegen: string-interpolation patterns not implemented".into());
		}
	}
	Ok(fails)
}

// Emit sub-patterns for a container pattern (Tuple/Record/Constructor/List),
// inserting per-sub trampolines that drain any orphaned payload items off
// the stack before jumping to the outer fail target.
//
// Background: a container match instruction (MatchTuple/MatchList/...) pushes
// its payload values onto the stack last-on-top. Sub-patterns are emitted in
// reverse so each consumes the value at the top. When sub-pattern k fails,
// the payload values for sub-patterns 0..k haven't been consumed yet — they
// would be orphans on the stack visible to the caller's fail handler. Each
// sub_fail jump is rewritten to land on a small "Pop N times; Jump" stub.
//
// `subs` yields sub-patterns in match (i.e. reverse) order. `total` is the
// number of sub-patterns (so we can compute orphan count per index).
fn emit_sub_patterns_with_cleanup<'a, I>(
	cg: &mut CodeGen,
	fb: &mut FunctionBuilder,
	scope: &mut Scope,
	subs: I,
	total: usize,
	range: Range,
	fails: &mut Vec<u32>,
) -> Result<(), String>
where
	I: IntoIterator<Item = &'a PatternNode>,
{
	for (rev_idx, sub) in subs.into_iter().enumerate() {
		let orphans = total - 1 - rev_idx;
		let sub_fails = emit_pattern(cg, fb, scope, &compiler::types::Type::Unknown, sub)?;
		if sub_fails.is_empty() || orphans == 0 {
			fails.extend(sub_fails);
			continue;
		}
		// Skip the trampoline on the normal-flow (success) path.
		let skip = fb.emit(Instruction::Jump(0), range);
		let tramp_start = fb.here();
		for sf in &sub_fails {
			fb.patch_jump(*sf, tramp_start);
		}
		for _ in 0..orphans {
			fb.emit(Instruction::Pop, range);
		}
		let final_jump = fb.emit(Instruction::Jump(0), range);
		fails.push(final_jump);
		let after = fb.here();
		fb.patch_jump(skip, after);
	}
	Ok(())
}

// --------------------------------------------------------------------------
// Misc.
// --------------------------------------------------------------------------

fn collect_enum_defs(
	module_name: &str,
	ast: &ModuleNode,
	out: &mut HashMap<String, Vec<(String, usize)>>,
) {
	for def in &ast.body {
		if let DefinitionKind::Enum(enum_node) = &def.kind {
			let qualified = format!("{}.{}", module_name, def.name.name);
			let variants: Vec<(String, usize)> = enum_node
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

fn regex_pattern(node: &RegexNode) -> String {
	match &node.kind {
		RegexKind::Literal(s) => regex::escape(s),
		RegexKind::CharacterClass(c) => match c.as_str() {
			"any" => ".".to_string(),
			"digit" => "[0-9]".to_string(),
			"letter" => "[A-Za-z]".to_string(),
			"whitespace" => "[ \\t\\n\\r]".to_string(),
			"word" => "[A-Za-z0-9_]".to_string(),
			// Analyzer rejects unknown names, so this is unreachable in
			// practice. Fall through to a literal that can't possibly
			// match so a buggy build at least fails closed.
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
