use crate::env::Environment;
use crate::eval::{builtin, expr::eval_expr};
use crate::value::{Builtin, Value};
use compiler::ast::{DefinitionKind, ExprNode, ModuleNode};
use compiler::{Compiler, Range};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Where `print` writes. Defaults to process stdout; tests swap in a buffer.
pub enum StdoutSink {
	Real,
	Buffer(Rc<RefCell<Vec<u8>>>),
}

impl StdoutSink {
	pub fn write_line(&self, s: &str) {
		match self {
			StdoutSink::Real => println!("{}", s),
			StdoutSink::Buffer(buf) => {
				let mut b = buf.borrow_mut();
				b.extend_from_slice(s.as_bytes());
				b.push(b'\n');
			}
		}
	}
}

pub struct RuntimeError {
	pub message: String,
	pub range: Option<Range>,
}

impl RuntimeError {
	pub fn new(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			range: None,
		}
	}

	pub fn at(mut self, range: Range) -> Self {
		self.range = Some(range);
		self
	}
}

// One slot per top-level def. Defs evaluate lazily on first access; cycles
// (e.g. `def x x`) trip the Evaluating state.
enum TopSlot<'ast> {
	Pending(&'ast ExprNode),
	Evaluating,
	Evaluated(Value<'ast>),
}

// One module's top-level def table. Keyed by bare name; cross-module lookups
// go through Interpreter::module_tops.
pub struct ModuleTops<'ast> {
	slots: HashMap<String, RefCell<TopSlot<'ast>>>,
	// enum name -> [(variant name, payload arity)], in declaration order.
	pub enums: HashMap<String, Vec<(String, usize)>>,
	// local import name -> fully-qualified module name. `use a.b.c` binds
	// `c -> "a.b.c"`; `use a.b.c as x` binds `x -> "a.b.c"`.
	pub imports: HashMap<String, String>,
	// Pre-built runtime values for native (stdlib) modules. Empty for
	// user-parsed modules. Looked up before falling through to `slots`.
	pub native_values: HashMap<String, Value<'ast>>,
}

impl<'ast> ModuleTops<'ast> {
	fn from_ast(ast: &'ast ModuleNode) -> Self {
		let mut slots = HashMap::new();
		let mut enums = HashMap::new();
		let mut imports = HashMap::new();
		for use_node in &ast.uses {
			imports.insert(use_node.local_name().name.clone(), use_node.module_name());
		}
		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(expr) => {
					slots.insert(def.name.name.clone(), RefCell::new(TopSlot::Pending(expr)));
				}
				DefinitionKind::Enum(enum_node) => {
					let variants: Vec<(String, usize)> = enum_node
						.variants
						.iter()
						.map(|v| (v.name.name.clone(), v.params.as_ref().map_or(0, |p| p.len())))
						.collect();
					enums.insert(def.name.name.clone(), variants);
				}
				DefinitionKind::Alias(_) => {
					// Alias defs don't produce runtime values until we add
					// alias constructor support; skip for now.
				}
			}
		}
		Self {
			slots,
			enums,
			imports,
			native_values: HashMap::new(),
		}
	}

	pub fn from_natives(values: HashMap<String, Value<'ast>>) -> Self {
		Self {
			slots: HashMap::new(),
			enums: HashMap::new(),
			imports: HashMap::new(),
			native_values: values,
		}
	}

	pub fn has_def(&self, name: &str) -> bool {
		self.slots.contains_key(name) || self.native_values.contains_key(name)
	}
}

pub struct Interpreter<'ast> {
	pub module_tops: HashMap<String, ModuleTops<'ast>>,
	pub entry_module: String,
	pub stdout: StdoutSink,
}

impl<'ast> Interpreter<'ast> {
	pub fn new(compiler: &'ast Compiler) -> Self {
		let mut module_tops = HashMap::new();
		for (name, module) in &compiler.modules {
			if let Some(ast) = &module.ast {
				module_tops.insert(name.clone(), ModuleTops::from_ast(ast));
			}
		}
		Self {
			module_tops,
			entry_module: compiler.entry_module_name.clone(),
			stdout: StdoutSink::Real,
		}
	}

	pub fn with_stdout(mut self, sink: StdoutSink) -> Self {
		self.stdout = sink;
		self
	}

	// Register a native (stdlib) module's runtime values. Must be called
	// before `run()`. Mirrors `Compiler::register_native_module` on the
	// types side.
	pub fn register_native_module(&mut self, name: String, values: HashMap<String, Value<'ast>>) {
		self.module_tops.insert(name, ModuleTops::from_natives(values));
	}

	// Resolve a top-level def by name in the given module. Forces evaluation
	// (memoized) on first access.
	pub fn force_top(
		&self,
		module_name: &str,
		def_name: &str,
	) -> Result<Value<'ast>, RuntimeError> {
		let tops = self.module_tops.get(module_name).ok_or_else(|| {
			RuntimeError::new(format!("unknown module `{}`", module_name))
		})?;
		// Native stdlib values short-circuit lazy evaluation.
		if let Some(v) = tops.native_values.get(def_name) {
			return Ok(v.clone());
		}
		let slot_cell = tops.slots.get(def_name).ok_or_else(|| {
			RuntimeError::new(format!("`{}.{}` is not defined", module_name, def_name))
		})?;

		// Quick path: already evaluated.
		if let TopSlot::Evaluated(v) = &*slot_cell.borrow() {
			return Ok(v.clone());
		}

		// Take the pending expr, marking the slot Evaluating so cycles are
		// caught.
		let expr = {
			let mut slot = slot_cell.borrow_mut();
			match &*slot {
				TopSlot::Evaluating => {
					return Err(RuntimeError::new(format!(
						"cycle detected while evaluating `{}.{}`",
						module_name, def_name
					)))
				}
				TopSlot::Evaluated(v) => return Ok(v.clone()),
				TopSlot::Pending(expr) => {
					let e = *expr;
					*slot = TopSlot::Evaluating;
					e
				}
			}
		};

		let mut env = self.prelude_env();
		let value = eval_expr(self, &mut env, module_name, expr)?;
		*slot_cell.borrow_mut() = TopSlot::Evaluated(value.clone());
		Ok(value)
	}

	// Each module sees the prelude in its outermost env scope.
	pub fn prelude_env(&self) -> Environment<'ast> {
		let mut env = Environment::new();
		env.define("print".into(), Value::Builtin(Builtin::Print));
		env.define("to-string".into(), Value::Builtin(Builtin::ToString));
		env
	}

	// Locate `main` in the entry module, ensure it's a zero-arg function, and
	// call it.
	pub fn run(&self) -> Result<Value<'ast>, RuntimeError> {
		let entry = self.entry_module.clone();
		let main = self.force_top(&entry, "main")?;
		match &main {
			Value::Closure { params, .. } if params.is_empty() => {
				apply(self, main, vec![Value::Nothing], Range::collapsed(0, 0), &entry)
			}
			Value::Closure { .. } => Err(RuntimeError::new(
				"`main` must be a zero-argument function",
			)),
			_ => Err(RuntimeError::new("`main` must be a function")),
		}
	}
}

// Used by eval to apply builtins / closures uniformly.
pub fn apply<'ast>(
	interp: &Interpreter<'ast>,
	callee: Value<'ast>,
	args: Vec<Value<'ast>>,
	call_range: Range,
	current_module: &str,
) -> Result<Value<'ast>, RuntimeError> {
	match callee {
		Value::Closure {
			params,
			body,
			env: captured_env,
			defining_module,
		} => {
			// The analyzer treats `fun {}` as `nothing -> X`, so it's called
			// as `f ()`. At the value level a zero-param closure receiving a
			// single `()` arg is a zero-arg invocation — discard the arg.
			let args = if params.is_empty() && args.len() == 1 && matches!(args[0], Value::Nothing)
			{
				Vec::new()
			} else {
				args
			};
			if params.len() != args.len() {
				return Err(RuntimeError::new(format!(
					"arity mismatch: expected {} args, got {}",
					params.len(),
					args.len()
				))
				.at(call_range));
			}
			let mut env = captured_env;
			env.enter_scope();
			for (name, value) in params.into_iter().zip(args.into_iter()) {
				env.define(name, value);
			}
			// Body evaluates in the module the closure was defined in, not
			// the caller's module — so free identifiers in the body resolve
			// against the right top-level defs.
			let _ = current_module;
			let mut last = Value::Nothing;
			for e in body {
				last = eval_expr(interp, &mut env, &defining_module, e)?;
			}
			env.leave_scope();
			Ok(last)
		}
		Value::Builtin(b) => builtin::call(interp, b, args, call_range),
		Value::VariantCtor {
			qualified_enum,
			variant,
			arity,
		} => {
			if args.len() != arity {
				return Err(RuntimeError::new(format!(
					"variant `{}.{}` takes {} arg(s), got {}",
					qualified_enum.rsplit_once('.').map(|(_, n)| n).unwrap_or(&qualified_enum),
					variant,
					arity,
					args.len()
				))
				.at(call_range));
			}
			Ok(Value::Variant {
				qualified_enum,
				variant,
				payload: args,
			})
		}
		_ => Err(RuntimeError::new("not callable").at(call_range)),
	}
}

