// AST → bytecode lowering.
//
// Single-pass walk of all loaded modules + the prelude. The CodeGen struct
// owns the in-progress Program and tracks per-function scope while compiling
// expressions. Identifier resolution distinguishes locals, captures, and
// globals; closures capture their free vars explicitly.

use compiler::ast::{
	CallNode, DefinitionKind, ExprKind, ExprNode, FunNode, IfNode, IdentifierNode, LetNode,
	LiteralKind, ModuleNode, Operator, PatternKind, PatternNode, RegexKind, RegexNode, WhenNode,
	WhileNode,
};
use compiler::Range;
use std::collections::HashMap;
use std::rc::Rc;
use vm::{
	native_modules, Builtin, Function, GlobalIdx, Instruction, Program, RegexData, SlotIdx, Value,
};

pub fn compile(compiler: &compiler::Compiler) -> Result<Program, String> {
	let mut cg = CodeGen::new();

	// Prelude: `print` and `to-string` as globals 0 and 1, pre-evaluated.
	cg.add_evaluated_global("__prelude__", "print", Value::Builtin(Builtin::Print));
	cg.add_evaluated_global("__prelude__", "to-string", Value::Builtin(Builtin::ToString));

	// Native modules: each def's value is a pre-evaluated Builtin.
	for module in native_modules() {
		for def in &module.defs {
			cg.add_evaluated_global(
				module.name,
				def.name,
				Value::Builtin(def.builtin),
			);
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

	// Build the entry function: load main, call it with (), return.
	let main_global = cg.lookup_global(&compiler.entry_module_name, "main").ok_or_else(|| {
		format!(
			"module `{}` has no `main` def",
			compiler.entry_module_name
		)
	})?;
	let entry_idx = cg.emit_entry_function(main_global);
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
				regex_patterns: Vec::new(),
				globals: Vec::new(),
				global_by_name: HashMap::new(),
				enum_variants: HashMap::new(),
				entry: 0,
			},
			const_lookup: HashMap::new(),
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
		self.program.globals.push(vm::program::GlobalSlot::Pending(0));
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
		let imports: HashMap<String, String> = ast
			.uses
			.iter()
			.map(|u| (u.local_name().name.clone(), u.module_name()))
			.collect();

		// Stash enum_variants from this module's enum defs into the Program.
		// (Already collected into self.enum_variants; flush into program for
		// the VM to use at runtime if needed.)
		for (k, v) in &self.enum_variants {
			self
				.program
				.enum_variants
				.insert(k.clone(), v.clone());
		}

		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(expr) => {
					let global_idx = self
						.lookup_global(module_name, &def.name.name)
						.expect("global slot reserved in pass 1");
					let fn_idx = self.compile_thunk(
						module_name,
						&imports,
						&format!("{}.{}", module_name, def.name.name),
						expr,
					)?;
					self.set_global_thunk(global_idx, fn_idx);
				}
				DefinitionKind::Alias(_) => {
					// Alias constructor: `fun x { x }`. Single-arg pass-through.
					let global_idx = self
						.lookup_global(module_name, &def.name.name)
						.expect("global slot reserved in pass 1");
					let alias_fn_idx = self.emit_alias_constructor(&def.name.name);
					// The "thunk" returns a closure over alias_fn_idx with no captures.
					let thunk_idx =
						self.emit_alias_thunk(&def.name.name, alias_fn_idx);
					self.set_global_thunk(global_idx, thunk_idx);
				}
				DefinitionKind::Enum(_) => {
					// Nothing to emit — enums show up at use sites as
					// MakeVariant or MakeVariantCtor based on their variant
					// shape, looked up via self.enum_variants.
				}
			}
		}

		Ok(())
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
		let mut fb = FunctionBuilder::new(name.to_string(), 0);
		let mut scope = Scope::new();
		emit_expr(self, current_module, imports, &mut fb, &mut scope, expr, true)?;
		fb.emit(Instruction::Return, expr.range);
		Ok(self.add_function(fb))
	}

	fn emit_alias_constructor(&mut self, alias_name: &str) -> u32 {
		let mut fb = FunctionBuilder::new(format!("alias-ctor:{}", alias_name), 1);
		fb.slot_count = 1;
		fb.emit(Instruction::LoadLocal(0), Range::collapsed(0, 0));
		fb.emit(Instruction::Return, Range::collapsed(0, 0));
		self.add_function(fb)
	}

	fn emit_alias_thunk(&mut self, alias_name: &str, alias_fn_idx: u32) -> u32 {
		let mut fb = FunctionBuilder::new(format!("alias-thunk:{}", alias_name), 0);
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
		let mut fb = FunctionBuilder::new("__entry__".into(), 0);
		fb.emit(Instruction::LoadGlobal(main_global), Range::collapsed(0, 0));
		fb.emit(Instruction::LoadNothing, Range::collapsed(0, 0));
		// Tail-call so main runs in our frame rather than nested under it.
		fb.emit(Instruction::TailCall(1), Range::collapsed(0, 0));
		fb.emit(Instruction::Return, Range::collapsed(0, 0));
		self.add_function(fb)
	}

	fn add_function(&mut self, fb: FunctionBuilder) -> u32 {
		let idx = self.program.functions.len() as u32;
		self.program.functions.push(Function {
			name: fb.name,
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
	param_count: u16,
	slot_count: u16,
	capture_count: u16,
	body: Vec<Instruction>,
	source_ranges: Vec<Range>,
}

impl FunctionBuilder {
	fn new(name: String, param_count: u16) -> Self {
		Self {
			name,
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
			Instruction::Jump(o)
			| Instruction::JumpIfFalse(o) => *o = target,
			Instruction::MatchInt(_, o)
			| Instruction::MatchFloat(_, o)
			| Instruction::MatchString(_, o)
			| Instruction::MatchBool(_, o)
			| Instruction::MatchNothing(o)
			| Instruction::MatchVariant { on_fail: o, .. }
			| Instruction::MatchTuple { on_fail: o, .. }
			| Instruction::MatchRecord { on_fail: o, .. } => *o = target,
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
	None
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
	emit_expr_with_parents(cg, current_module, imports, fb, scope, &mut Vec::new(), expr, tail)
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
			emit_identifier(cg, current_module, imports, fb, scope, parent_scopes, ident, range)?;
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
		ExprKind::Let(LetNode { name, value, .. }) => {
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
			let slot = fb.alloc_slot();
			fb.emit(Instruction::StoreLocal(slot), range);
			scope.define_local(&name.name, slot);
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
		ExprKind::List(elems) => {
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
			fb.emit(Instruction::MakeList(elems.len() as u16), range);
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
			fb.emit(
				Instruction::MakeRecord {
					fields: field_idxs,
				},
				range,
			);
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
			emit_fun(cg, current_module, imports, fb, scope, parent_scopes, params, body, range)?;
		}
		ExprKind::Call(CallNode { callee, args, .. }) => {
			emit_call(
				cg,
				current_module,
				imports,
				fb,
				scope,
				parent_scopes,
				callee,
				args,
				range,
				tail,
			)?;
		}
		ExprKind::FieldAccess { receiver, field } => {
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
		ExprKind::BinaryOperation { op, left, right } => {
			emit_expr_with_parents(
				cg, current_module, imports, fb, scope, parent_scopes, left, false,
			)?;
			emit_expr_with_parents(
				cg, current_module, imports, fb, scope, parent_scopes, right, false,
			)?;
			let is_float =
				matches!(left.ty, compiler::types::Type::Float)
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
			emit_expr_with_parents(
				cg, current_module, imports, fb, scope, parent_scopes, right, false,
			)?;
			let is_float = matches!(right.ty, compiler::types::Type::Float);
			let instr = match (op, is_float) {
				(Operator::SubtractionOrNegation, false) => Instruction::NegInt,
				(Operator::SubtractionOrNegation, true) => Instruction::NegFloat,
				(Operator::LogicalNot, _) => Instruction::LogicalNot,
				_ => return Err(format!("codegen: unhandled unary op {}", op)),
			};
			fb.emit(instr, range);
		}
		ExprKind::Regex(node) => {
			let pattern = regex_pattern(node);
			let compiled = regex::Regex::new(&pattern)
				.map_err(|e| format!("codegen: invalid regex: {}", e))?;
			let idx = cg.program.regex_patterns.len() as u32;
			cg.program.regex_patterns.push(Rc::new(RegexData { compiled }));
			fb.emit(Instruction::LoadRegex(idx), range);
		}
		ExprKind::If(IfNode {
			subject,
			pattern,
			body,
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
				range,
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
		ExprKind::ElementAccess { .. } => {
			return Err("codegen: ElementAccess not implemented".into());
		}
	}
	Ok(())
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
		LiteralKind::FloatDecimal(f) => {
			fb.emit(Instruction::LoadFloat(*f), range);
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
	let mut parent_refs: Vec<&mut Scope> = parent_scopes
		.iter()
		.map(|p| unsafe { &mut **p })
		.collect();
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
		Resolution::EnumName(_) | Resolution::Imported(_) => {
			return Err(format!(
				"codegen: `{}` is a namespace, not a value",
				ident.name
			));
		}
	}
	Ok(())
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
	fb.emit(
		Instruction::MakeClosure {
			fn_idx: inner_fn_idx,
			num_captures: captures.len() as u16,
		},
		range,
	);
	Ok(())
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
	range: Range,
	tail: bool,
) -> Result<(), String> {
	emit_expr_with_parents(
		cg, current_module, imports, fb, scope, parent_scopes, callee, false,
	)?;
	for a in args {
		emit_expr_with_parents(
			cg, current_module, imports, fb, scope, parent_scopes, a, false,
		)?;
	}
	let instr = if tail {
		Instruction::TailCall(args.len() as u16)
	} else {
		Instruction::Call(args.len() as u16)
	};
	fb.emit(instr, range);
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
						return emit_variant_construction(
							cg,
							fb,
							&qualified_enum,
							&field.name,
							*arity,
							range,
						);
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
				return emit_variant_construction(
					cg,
					fb,
					&qualified_enum,
					&field.name,
					*arity,
					range,
				);
			}
		}
	}

	// 4. Record field access.
	emit_expr_with_parents(
		cg, current_module, imports, fb, scope, parent_scopes, receiver, false,
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
	range: Range,
) -> Result<(), String> {
	// `if X is P { body }` — match, run body, else skip. Always evaluates
	// to nothing — so the body's expressions are never in tail position
	// (their values get popped).
	emit_expr_with_parents(
		cg, current_module, imports, fb, scope, parent_scopes, subject, false,
	)?;
	let subject_ty = subject.ty.clone();
	scope.enter();
	let fail_idx = emit_pattern(cg, fb, scope, &subject_ty, pattern)?;
	for e in body {
		emit_expr_with_parents(
			cg, current_module, imports, fb, scope, parent_scopes, e, false,
		)?;
		fb.emit(Instruction::Pop, e.range);
	}
	let end_jump = fb.emit(Instruction::Jump(0), range);
	let fail_target = fb.here();
	for fi in fail_idx {
		fb.patch_jump(fi, fail_target);
	}
	let end = fb.here();
	fb.patch_jump(end_jump, end);
	scope.leave();
	fb.emit(Instruction::LoadNothing, range);
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
		cg, current_module, imports, fb, scope, parent_scopes, subject, false,
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
		cg, current_module, imports, fb, scope, parent_scopes, subject, false,
	)?;
	let subject_ty = subject.ty.clone();
	scope.enter();
	let fail_idx = emit_pattern(cg, fb, scope, &subject_ty, pattern)?;
	for e in body {
		emit_expr_with_parents(
			cg, current_module, imports, fb, scope, parent_scopes, e, false,
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
			let is_variant_match = if let compiler::types::Type::Enum(qualified) = subject_ty {
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
				LiteralKind::FloatDecimal(f) => fb.emit(Instruction::MatchFloat(*f, 0), range),
				LiteralKind::IntDecimal(n)
				| LiteralKind::IntHex(n)
				| LiteralKind::IntOctal(n)
				| LiteralKind::IntBinary(n) => {
					fb.emit(Instruction::MatchInt(*n as i64, 0), range)
				}
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
			for sub in elems.iter().rev() {
				let sub_fails = emit_pattern(cg, fb, scope, &compiler::types::Type::Unknown, sub)?;
				fails.extend(sub_fails);
			}
		}
		PatternKind::Record(fields) => {
			let field_idxs: Vec<u32> = fields.iter().map(|(n, _)| cg.intern(&n.name)).collect();
			let jmp = fb.emit(
				Instruction::MatchRecord {
					fields: field_idxs,
					on_fail: 0,
				},
				range,
			);
			fails.push(jmp);
			for (_, sub) in fields.iter().rev() {
				let sub_fails = emit_pattern(cg, fb, scope, &compiler::types::Type::Unknown, sub)?;
				fails.extend(sub_fails);
			}
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
			for sub in sub_patterns.iter().rev() {
				let sub_fails = emit_pattern(cg, fb, scope, &compiler::types::Type::Unknown, sub)?;
				fails.extend(sub_fails);
			}
		}
		PatternKind::Interpolation(_) => {
			return Err("codegen: string-interpolation patterns not implemented".into());
		}
	}
	Ok(fails)
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
				.map(|v| (v.name.name.clone(), v.params.as_ref().map_or(0, |p| p.len())))
				.collect();
			out.insert(qualified, variants);
		}
	}
}

fn regex_pattern(node: &RegexNode) -> String {
	match &node.kind {
		RegexKind::Literal(s) => regex::escape(s),
		RegexKind::CharacterClass(c) => format!("[{}]", c),
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
