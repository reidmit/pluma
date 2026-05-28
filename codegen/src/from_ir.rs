// IR -> bytecode lowering: the second consumer of `ir::IrProgram` (the bytecode
// VM, via this translation; a future WASM backend would be the third).
//
// Translates the target-independent IR into a `vm::Program`. Because the IR is
// ANF + structured control flow, this is a mechanical pass: each `Rvalue`/
// `Stmt` emits a short, local instruction sequence. Storage is assigned here
// (the VM's `base + slot` locals), not in the IR — `VarId`s become stack slots,
// captures become `LoadCapture` indices.
//
// Phase 1.2, growing alongside `ir::lower`. Covers the node set the lowering
// produces so far: literals/atoms, global refs, closures, calls, binary/unary
// ops, dict-method dispatch, variant construction (+ constructors), tuples,
// records, lists (with spread), interpolation, field access, regex literals,
// structured control flow (`If`/`Loop`/`Break`/`Continue`) and the pattern
// `Match`.
// Unsupported nodes return an `Err` rather than emitting wrong bytecode, so
// gaps surface loudly.

use std::collections::HashMap;
use std::rc::Rc;

use compiler::Range;
use ir::{
	Atom, BinOp, Block, Callee, Const, Function as IrFunction, GlobalInit, IrProgram, ListItem,
	ListRest, Pattern, PreEval, RecordRest, Rvalue, Stmt, StmtKind, VarId,
};
use vm::program::GlobalSlot;
use vm::{Function, Instruction, Program, RegexData, Value};

/// Lower a complete IR program to a runnable `vm::Program`.
///
/// IR `FuncId`s are assumed dense and in `functions` order, so a `FuncId(n)`
/// maps to VM function index `n`; the emitter preserves that order.
pub fn emit(program: &IrProgram) -> Result<Program, String> {
	let mut e = Emitter::default();
	e.enums = program.enums.clone();
	// `FuncId(n)` indexes `functions[n]`, so a parallel async flag table lets
	// `MakeClosure` over an async function emit `MakeAsyncClosure` instead.
	e.fn_is_async = program.functions.iter().map(|f| f.is_async).collect();
	for func in &program.functions {
		let f = e.lower_function(func)?;
		e.functions.push(f);
	}
	let globals = program.globals.iter().map(lower_global).collect();
	Ok(Program {
		functions: e.functions,
		constants: e.constants,
		bytes_constants: e.bytes_constants,
		regex_patterns: e.regex_patterns,
		globals,
		field_lists: e.field_lists,
		// Only used by codegen-time tooling / the test runner, never read by the
		// VM at runtime. Left empty until `ir::lower` carries global names.
		global_by_name: HashMap::new(),
		enum_variants: program.enums.clone(),
		entry: program.entry.0,
		test_suites: program
			.test_suites
			.iter()
			.map(|(m, g)| (m.clone(), g.0))
			.collect(),
		test_new: None,
	})
}

#[derive(Default)]
struct Emitter {
	functions: Vec<Function>,
	constants: Vec<Rc<String>>,
	const_lookup: HashMap<String, u32>,
	bytes_constants: Vec<Rc<Vec<u8>>>,
	bytes_lookup: HashMap<Vec<u8>, u32>,
	// Compiled regex literals, indexed by the LoadRegex operand.
	regex_patterns: Vec<Rc<RegexData>>,
	// Record-shape field-name lists, indexed by FieldListIdx.
	field_lists: Vec<Vec<u32>>,
	// qualified-enum-name -> [(variant_name, arity)], for resolving a
	// `MakeVariant`/`MakeVariantCtor` tag back to the variant name the VM wants.
	enums: HashMap<String, Vec<(String, usize)>>,
	// Per-`FuncId` async flag (dense, in `functions` order). A `MakeClosure`
	// targeting an async function emits `MakeAsyncClosure` (-> `Value::AsyncFn`).
	fn_is_async: Vec<bool>,
}

impl Emitter {
	fn intern(&mut self, s: &str) -> u32 {
		if let Some(&idx) = self.const_lookup.get(s) {
			return idx;
		}
		let idx = self.constants.len() as u32;
		self.constants.push(Rc::new(s.to_string()));
		self.const_lookup.insert(s.to_string(), idx);
		idx
	}

	fn intern_bytes(&mut self, b: &[u8]) -> u32 {
		if let Some(&idx) = self.bytes_lookup.get(b) {
			return idx;
		}
		let idx = self.bytes_constants.len() as u32;
		self.bytes_constants.push(Rc::new(b.to_vec()));
		self.bytes_lookup.insert(b.to_vec(), idx);
		idx
	}

	fn intern_field_list(&mut self, fields: Vec<u32>) -> u32 {
		let idx = self.field_lists.len() as u32;
		self.field_lists.push(fields);
		idx
	}

	fn lower_function(&mut self, f: &IrFunction) -> Result<Function, String> {
		let mut ctx = FnCtx::new(f);
		let mut body = Vec::new();
		let mut ranges = Vec::new();
		ctx.lower_block(self, &f.body, &mut body, &mut ranges)?;
		Ok(Function {
			name: f.name.clone(),
			module: f.module.clone(),
			param_count: f.params.len() as u16,
			slot_count: ctx.slot_count,
			capture_count: f.captures.len() as u16,
			body,
			source_ranges: ranges,
		})
	}
}

/// Where a `VarId` lives within a single function: a stack-local slot (params
/// and `let`s) or a closure capture index.
enum Loc {
	Local(u16),
	Capture(u16),
}

/// Per-function lowering context: the `VarId` -> location map and the local
/// slot count. Captures don't consume slots (they live in the frame's capture
/// array); `slot_count` is params + `let`s.
struct FnCtx {
	locs: HashMap<u32, Loc>,
	slot_count: u16,
	// Active loop frames (innermost last): `start` is the loop-top instruction
	// index (for `Continue`); `breaks` collects `Break` jump indices to patch
	// to the loop exit.
	loops: Vec<LoopFrame>,
}

struct LoopFrame {
	start: u32,
	breaks: Vec<u32>,
}

impl FnCtx {
	fn new(f: &IrFunction) -> Self {
		let mut locs = HashMap::new();
		let mut slot = 0u16;
		for p in &f.params {
			locs.insert(p.0, Loc::Local(slot));
			slot += 1;
		}
		for (i, c) in f.captures.iter().enumerate() {
			locs.insert(c.0, Loc::Capture(i as u16));
		}
		let mut ctx = FnCtx {
			locs,
			slot_count: slot,
			loops: Vec::new(),
		};
		ctx.assign_let_slots(&f.body);
		ctx
	}

	fn assign_slot(&mut self, v: VarId) {
		if !self.locs.contains_key(&v.0) {
			let s = self.slot_count;
			self.slot_count += 1;
			self.locs.insert(v.0, Loc::Local(s));
		}
	}

	/// Pre-assign a local slot to every `let`-bound `VarId`, descending into
	/// nested blocks. ANF guarantees a var is bound before use, so a single
	/// pre-pass suffices.
	fn assign_let_slots(&mut self, block: &Block) {
		for stmt in &block.0 {
			match &stmt.kind {
				StmtKind::Let(v, _) => self.assign_slot(*v),
				StmtKind::If(_, t, e) => {
					self.assign_let_slots(t);
					self.assign_let_slots(e);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, b) in arms {
						self.assign_let_slots(b);
					}
					self.assign_let_slots(default);
				}
				StmtKind::Loop(b) => self.assign_let_slots(b),
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						self.assign_pattern_slots(&arm.pattern);
						self.assign_let_slots(&arm.body);
					}
				}
				_ => {}
			}
		}
	}

	/// Assign slots to every variable a pattern binds, descending into nested
	/// sub-patterns.
	fn assign_pattern_slots(&mut self, pat: &Pattern) {
		match pat {
			Pattern::Bind(v) => self.assign_slot(*v),
			Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
				for f in fields {
					self.assign_pattern_slots(f);
				}
			}
			Pattern::List { items, rest } => {
				for i in items {
					self.assign_pattern_slots(i);
				}
				if let Some(ListRest::Bind(v)) = rest {
					self.assign_slot(*v);
				}
			}
			Pattern::Record { fields, rest } => {
				for (_, p) in fields {
					self.assign_pattern_slots(p);
				}
				if let RecordRest::Bind(v) = rest {
					self.assign_slot(*v);
				}
			}
			Pattern::Wildcard | Pattern::Literal(_) => {}
		}
	}

	fn loc(&self, v: VarId) -> Result<&Loc, String> {
		self
			.locs
			.get(&v.0)
			.ok_or_else(|| format!("from_ir: unbound VarId({})", v.0))
	}

	fn local_slot(&self, v: VarId) -> Result<u16, String> {
		match self.loc(v)? {
			Loc::Local(s) => Ok(*s),
			Loc::Capture(_) => Err(format!("from_ir: cannot store into capture VarId({})", v.0)),
		}
	}

	fn lower_block(
		&mut self,
		em: &mut Emitter,
		block: &Block,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
	) -> Result<(), String> {
		for stmt in &block.0 {
			self.lower_stmt(em, stmt, body, ranges)?;
		}
		Ok(())
	}

	fn lower_stmt(
		&mut self,
		em: &mut Emitter,
		stmt: &Stmt,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
	) -> Result<(), String> {
		// Every instruction emitted directly while lowering this stmt inherits
		// `r` — only recursing into a nested `Block` re-derives the range from
		// its own inner stmts. That's enough for `debug`'s call-site reporting
		// and for runtime errors to point at the producing source line.
		let r = stmt.range;
		match &stmt.kind {
			StmtKind::Let(v, rv) => {
				self.lower_rvalue(em, rv, body, ranges, r)?;
				let slot = self.local_slot(*v)?;
				push(body, ranges, Instruction::StoreLocal(slot), r);
			}
			StmtKind::Discard(rv) => {
				self.lower_rvalue(em, rv, body, ranges, r)?;
				push(body, ranges, Instruction::Pop, r);
			}
			StmtKind::Return(atom) => {
				self.lower_atom(em, atom, body, ranges, r)?;
				push(body, ranges, Instruction::Return, r);
			}
			StmtKind::If(cond, then_b, else_b) => {
				self.lower_atom(em, cond, body, ranges, r)?;
				let jf = emit_at(body, ranges, Instruction::JumpIfFalse(0), r);
				self.lower_block(em, then_b, body, ranges)?;
				let j_end = emit_at(body, ranges, Instruction::Jump(0), r);
				let else_start = body.len() as u32;
				patch(body, jf, else_start);
				self.lower_block(em, else_b, body, ranges)?;
				let end = body.len() as u32;
				patch(body, j_end, end);
			}
			StmtKind::Loop(b) => {
				let start = body.len() as u32;
				self.loops.push(LoopFrame {
					start,
					breaks: Vec::new(),
				});
				self.lower_block(em, b, body, ranges)?;
				push(body, ranges, Instruction::Jump(start), r);
				let frame = self.loops.pop().expect("loop frame");
				let end = body.len() as u32;
				for bj in frame.breaks {
					patch(body, bj, end);
				}
			}
			StmtKind::Break => {
				let j = emit_at(body, ranges, Instruction::Jump(0), r);
				self
					.loops
					.last_mut()
					.ok_or("from_ir: break outside loop")?
					.breaks
					.push(j);
			}
			StmtKind::Continue => {
				let start = self
					.loops
					.last()
					.ok_or("from_ir: continue outside loop")?
					.start;
				push(body, ranges, Instruction::Jump(start), r);
			}
			StmtKind::PushDefer(closure) => {
				self.lower_atom(em, closure, body, ranges, r)?;
				push(body, ranges, Instruction::PushDefer, r);
			}
			StmtKind::Match { subject, arms } => {
				let mut end_jumps = Vec::new();
				for arm in arms {
					self.lower_atom(em, subject, body, ranges, r)?;
					let fails = self.emit_pattern(em, &arm.pattern, body, ranges, r)?;
					self.lower_block(em, &arm.body, body, ranges)?;
					end_jumps.push(emit_at(body, ranges, Instruction::Jump(0), r));
					let next = body.len() as u32;
					for f in fails {
						patch(body, f, next);
					}
				}
				let end = body.len() as u32;
				for j in end_jumps {
					patch(body, j, end);
				}
			}
			other => return Err(format!("from_ir: unsupported statement: {other:?}")),
		}
		Ok(())
	}

	/// Emit the match test for `pattern` against the subject currently on top
	/// of the stack. Returns the indices of any fail-jump instructions (to be
	/// patched to the next arm). On a successful variant match the payload is
	/// bound/popped; `Wildcard`/`Bind` always succeed (no fail jump).
	fn emit_pattern(
		&self,
		em: &mut Emitter,
		pattern: &Pattern,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<Vec<u32>, String> {
		match pattern {
			Pattern::Wildcard => {
				push(body, ranges, Instruction::Pop, r);
				Ok(Vec::new())
			}
			Pattern::Bind(v) => {
				push(
					body,
					ranges,
					Instruction::StoreLocal(self.local_slot(*v)?),
					r,
				);
				Ok(Vec::new())
			}
			Pattern::Literal(c) => {
				let jmp = match c {
					Const::Int(n) => emit_at(body, ranges, Instruction::MatchInt(*n, 0), r),
					Const::Bool(b) => emit_at(body, ranges, Instruction::MatchBool(*b, 0), r),
					Const::Float(f) => emit_at(body, ranges, Instruction::MatchFloat(*f, 0), r),
					Const::Str(s) => {
						let idx = em.intern(s);
						emit_at(body, ranges, Instruction::MatchString(idx, 0), r)
					}
					Const::Bytes(b) => {
						let idx = em.intern_bytes(b);
						emit_at(body, ranges, Instruction::MatchBytes(idx, 0), r)
					}
					Const::Unit => emit_at(body, ranges, Instruction::MatchNothing(0), r),
					Const::Duration(n) => emit_at(body, ranges, Instruction::MatchDuration(*n, 0), r),
				};
				Ok(vec![jmp])
			}
			Pattern::Variant { variant, fields } => {
				let v = em.intern(variant);
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchVariant {
						variant: v,
						arity: fields.len() as u16,
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				self.emit_sub_patterns(em, fields, body, ranges, &mut fails, r)?;
				Ok(fails)
			}
			Pattern::Tuple(elems) => {
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchTuple {
						arity: elems.len() as u16,
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				self.emit_sub_patterns(em, elems, body, ranges, &mut fails, r)?;
				Ok(fails)
			}
			Pattern::List { items, rest } => {
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchList {
						arity: items.len() as u16,
						has_rest: rest.is_some(),
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				// The tail (if any) sits on top of the element values — consume
				// it before matching the elements in reverse.
				match rest {
					Some(ListRest::Bind(v)) => push(
						body,
						ranges,
						Instruction::StoreLocal(self.local_slot(*v)?),
						r,
					),
					Some(ListRest::Anon) => push(body, ranges, Instruction::Pop, r),
					None => {}
				}
				self.emit_sub_patterns(em, items, body, ranges, &mut fails, r)?;
				Ok(fails)
			}
			Pattern::Record { fields, rest } => {
				let idxs: Vec<u32> = fields.iter().map(|(n, _)| em.intern(n)).collect();
				let fields_idx = em.intern_field_list(idxs);
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchRecord {
						fields_idx,
						exact: matches!(rest, RecordRest::Exact),
						with_rest: matches!(rest, RecordRest::Bind(_)),
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				// With a bound rest, the rest record is on top — consume it first.
				if let RecordRest::Bind(v) = rest {
					push(
						body,
						ranges,
						Instruction::StoreLocal(self.local_slot(*v)?),
						r,
					);
				}
				let sub_pats: Vec<&Pattern> = fields.iter().map(|(_, p)| p).collect();
				self.emit_sub_patterns_refs(em, &sub_pats, body, ranges, &mut fails, r)?;
				Ok(fails)
			}
		}
	}

	/// Emit a container's sub-patterns (matched in reverse, since payload is
	/// pushed last-on-top), inserting cleanup trampolines: when sub-pattern `k`
	/// fails, the still-unconsumed payloads for the earlier elements are popped
	/// before jumping to the outer fail. Mirrors
	/// `codegen::emit::emit_sub_patterns_with_cleanup`.
	fn emit_sub_patterns(
		&self,
		em: &mut Emitter,
		subs: &[Pattern],
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		fails: &mut Vec<u32>,
		r: Range,
	) -> Result<(), String> {
		let refs: Vec<&Pattern> = subs.iter().collect();
		self.emit_sub_patterns_refs(em, &refs, body, ranges, fails, r)
	}

	fn emit_sub_patterns_refs(
		&self,
		em: &mut Emitter,
		subs: &[&Pattern],
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		fails: &mut Vec<u32>,
		r: Range,
	) -> Result<(), String> {
		let total = subs.len();
		for (rev_idx, sub) in subs.iter().rev().enumerate() {
			let orphans = total - 1 - rev_idx;
			let sub_fails = self.emit_pattern(em, sub, body, ranges, r)?;
			if sub_fails.is_empty() || orphans == 0 {
				fails.extend(sub_fails);
				continue;
			}
			// Success path skips the trampoline.
			let skip = emit_at(body, ranges, Instruction::Jump(0), r);
			let tramp_start = body.len() as u32;
			for sf in &sub_fails {
				patch(body, *sf, tramp_start);
			}
			for _ in 0..orphans {
				push(body, ranges, Instruction::Pop, r);
			}
			fails.push(emit_at(body, ranges, Instruction::Jump(0), r));
			let after = body.len() as u32;
			patch(body, skip, after);
		}
		Ok(())
	}

	fn lower_rvalue(
		&self,
		em: &mut Emitter,
		rv: &Rvalue,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<(), String> {
		match rv {
			Rvalue::Use(a) => self.lower_atom(em, a, body, ranges, r)?,
			Rvalue::Bin(op, a, b) => {
				self.lower_atom(em, a, body, ranges, r)?;
				self.lower_atom(em, b, body, ranges, r)?;
				push(body, ranges, binop_instr(*op), r);
			}
			Rvalue::Not(a) => {
				self.lower_atom(em, a, body, ranges, r)?;
				push(body, ranges, Instruction::LogicalNot, r);
			}
			Rvalue::GetDictMethod(dict, idx) => {
				self.lower_atom(em, dict, body, ranges, r)?;
				push(body, ranges, Instruction::GetDictField(*idx as u16), r);
			}
			Rvalue::MakeDict(methods) => {
				for m in methods {
					self.lower_atom(em, m, body, ranges, r)?;
				}
				push(body, ranges, Instruction::MakeDict(methods.len() as u16), r);
			}
			Rvalue::MakeVariant {
				enum_name,
				tag,
				payload,
			} => {
				let (variant_name, _arity) = em
					.enums
					.get(enum_name)
					.and_then(|vs| vs.get(*tag as usize))
					.ok_or_else(|| format!("from_ir: unknown variant {enum_name}#{tag}"))?
					.clone();
				for a in payload {
					self.lower_atom(em, a, body, ranges, r)?;
				}
				let qualified = em.intern(enum_name);
				let variant = em.intern(&variant_name);
				push(
					body,
					ranges,
					Instruction::MakeVariant {
						qualified,
						variant,
						arity: payload.len() as u16,
					},
					r,
				);
			}
			Rvalue::GlobalRef(g) => push(body, ranges, Instruction::LoadGlobal(g.0), r),
			Rvalue::MakeClosure(fid, caps) => {
				for c in caps {
					self.lower_atom(em, c, body, ranges, r)?;
				}
				let num_captures = caps.len() as u16;
				let fn_idx = fid.0;
				// An async-bearing function (one whose body awaits a task) becomes a
				// `Value::AsyncFn`: calling it builds a cold task instead of running.
				let instr = if em
					.fn_is_async
					.get(fn_idx as usize)
					.copied()
					.unwrap_or(false)
				{
					Instruction::MakeAsyncClosure {
						fn_idx,
						num_captures,
					}
				} else {
					Instruction::MakeClosure {
						fn_idx,
						num_captures,
					}
				};
				push(body, ranges, instr, r);
			}
			Rvalue::CallClosure(callee, args) => {
				self.lower_atom(em, callee, body, ranges, r)?;
				for a in args {
					self.lower_atom(em, a, body, ranges, r)?;
				}
				push(body, ranges, Instruction::Call(args.len() as u16), r);
			}
			Rvalue::Call(callee, args) => {
				match callee {
					Callee::Global(g) => push(body, ranges, Instruction::LoadGlobal(g.0), r),
					Callee::Function(f) => push(
						body,
						ranges,
						Instruction::MakeClosure {
							fn_idx: f.0,
							num_captures: 0,
						},
						r,
					),
					Callee::Builtin(_) => {
						return Err("from_ir: Callee::Builtin not yet supported".to_string())
					}
				}
				for a in args {
					self.lower_atom(em, a, body, ranges, r)?;
				}
				push(body, ranges, Instruction::Call(args.len() as u16), r);
			}
			Rvalue::MakeVariantCtor { enum_name, tag } => {
				let (variant_name, arity) = em
					.enums
					.get(enum_name)
					.and_then(|vs| vs.get(*tag as usize))
					.ok_or_else(|| format!("from_ir: unknown variant {enum_name}#{tag}"))?
					.clone();
				let qualified = em.intern(enum_name);
				let variant = em.intern(&variant_name);
				push(
					body,
					ranges,
					Instruction::MakeVariantCtor {
						qualified,
						variant,
						arity: arity as u16,
					},
					r,
				);
			}
			Rvalue::MakeTuple(elems) => {
				for a in elems {
					self.lower_atom(em, a, body, ranges, r)?;
				}
				push(body, ranges, Instruction::MakeTuple(elems.len() as u16), r);
			}
			Rvalue::MakeRecord(fields) => {
				let mut idxs = Vec::with_capacity(fields.len());
				for (name, value) in fields {
					self.lower_atom(em, value, body, ranges, r)?;
					idxs.push(em.intern(name));
				}
				let fields_idx = em.intern_field_list(idxs);
				push(body, ranges, Instruction::MakeRecord(fields_idx), r);
			}
			Rvalue::MakeList(items) => self.lower_list(em, items, body, ranges, r)?,
			Rvalue::GetField(receiver, name) => {
				self.lower_atom(em, receiver, body, ranges, r)?;
				let idx = em.intern(name);
				push(body, ranges, Instruction::GetField(idx), r);
			}
			Rvalue::Interpolate(parts) => {
				for a in parts {
					self.lower_atom(em, a, body, ranges, r)?;
				}
				push(
					body,
					ranges,
					Instruction::Interpolate(parts.len() as u16),
					r,
				);
			}
			Rvalue::Regex(pattern) => {
				let compiled = regex::Regex::new(pattern)
					.map_err(|e| format!("from_ir: invalid regex `{pattern}`: {e}"))?;
				let idx = em.regex_patterns.len() as u32;
				em.regex_patterns.push(Rc::new(RegexData { compiled }));
				push(body, ranges, Instruction::LoadRegex(idx), r);
			}
			Rvalue::Await(task) => {
				// Suspension point inside a task step function (each task-carrier
				// `try`). Push the awaited task; the driver snapshots the frame,
				// runs the task, and resumes here with its result on the stack.
				self.lower_atom(em, task, body, ranges, r)?;
				push(body, ranges, Instruction::Await, r);
			}
			other => return Err(format!("from_ir: unsupported rvalue: {other:?}")),
		}
		Ok(())
	}

	/// List literal lowering, mirroring `emit.rs`: a spread-free list is one
	/// `MakeList`; a lone `[...xs]` is just `xs`; otherwise each run of plain
	/// elements becomes a `MakeList` segment and each spread its own segment,
	/// joined by `ConcatLists`.
	fn lower_list(
		&self,
		em: &mut Emitter,
		items: &[ListItem],
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<(), String> {
		let any_spread = items.iter().any(|i| matches!(i, ListItem::Spread(_)));
		if !any_spread {
			for i in items {
				if let ListItem::Elem(a) = i {
					self.lower_atom(em, a, body, ranges, r)?;
				}
			}
			push(body, ranges, Instruction::MakeList(items.len() as u16), r);
		} else if items.len() == 1 {
			if let ListItem::Spread(a) = &items[0] {
				self.lower_atom(em, a, body, ranges, r)?;
			}
		} else {
			let mut segments: u16 = 0;
			let mut run: u16 = 0;
			for i in items {
				match i {
					ListItem::Elem(a) => {
						self.lower_atom(em, a, body, ranges, r)?;
						run += 1;
					}
					ListItem::Spread(a) => {
						if run > 0 {
							push(body, ranges, Instruction::MakeList(run), r);
							segments += 1;
							run = 0;
						}
						self.lower_atom(em, a, body, ranges, r)?;
						segments += 1;
					}
				}
			}
			if run > 0 {
				push(body, ranges, Instruction::MakeList(run), r);
				segments += 1;
			}
			push(body, ranges, Instruction::ConcatLists(segments), r);
		}
		Ok(())
	}

	fn lower_atom(
		&self,
		em: &mut Emitter,
		atom: &Atom,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<(), String> {
		match atom {
			Atom::Var(v) => match self.loc(*v)? {
				Loc::Local(s) => push(body, ranges, Instruction::LoadLocal(*s), r),
				Loc::Capture(i) => push(body, ranges, Instruction::LoadCapture(*i), r),
			},
			Atom::Const(c) => match c {
				Const::Unit => push(body, ranges, Instruction::LoadNothing, r),
				Const::Bool(b) => push(body, ranges, Instruction::LoadBool(*b), r),
				Const::Int(n) => push(body, ranges, Instruction::LoadInt(*n), r),
				Const::Float(f) => push(body, ranges, Instruction::LoadFloat(*f), r),
				Const::Str(s) => {
					let idx = em.intern(s);
					push(body, ranges, Instruction::LoadConst(idx), r);
				}
				Const::Bytes(b) => {
					let idx = em.intern_bytes(b);
					push(body, ranges, Instruction::LoadBytes(idx), r);
				}
				Const::Duration(n) => push(body, ranges, Instruction::LoadDuration(*n), r),
			},
		}
		Ok(())
	}
}

fn push(body: &mut Vec<Instruction>, ranges: &mut Vec<Range>, instr: Instruction, range: Range) {
	body.push(instr);
	ranges.push(range);
}

/// Push an instruction and return its index (for later jump patching).
fn emit_at(
	body: &mut Vec<Instruction>,
	ranges: &mut Vec<Range>,
	instr: Instruction,
	range: Range,
) -> u32 {
	let idx = body.len() as u32;
	push(body, ranges, instr, range);
	idx
}

/// Patch the target offset of a jump-like instruction.
fn patch(body: &mut [Instruction], idx: u32, target: u32) {
	match &mut body[idx as usize] {
		Instruction::Jump(o)
		| Instruction::JumpIfFalse(o)
		| Instruction::MatchInt(_, o)
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
		other => panic!("from_ir patch: not a jump-like instruction: {other:?}"),
	}
}

fn binop_instr(op: BinOp) -> Instruction {
	match op {
		BinOp::AddInt => Instruction::AddInt,
		BinOp::SubInt => Instruction::SubInt,
		BinOp::MulInt => Instruction::MulInt,
		BinOp::DivInt => Instruction::DivInt,
		BinOp::RemInt => Instruction::RemInt,
		BinOp::AddFloat => Instruction::AddFloat,
		BinOp::SubFloat => Instruction::SubFloat,
		BinOp::MulFloat => Instruction::MulFloat,
		BinOp::DivFloat => Instruction::DivFloat,
		BinOp::RemFloat => Instruction::RemFloat,
		BinOp::Concat => Instruction::ConcatString,
		BinOp::And => Instruction::LogicalAnd,
		BinOp::Or => Instruction::LogicalOr,
		BinOp::Eq => Instruction::Eq,
		BinOp::Ne => Instruction::Neq,
		BinOp::Lt => Instruction::Lt,
		BinOp::Le => Instruction::Lte,
		BinOp::Gt => Instruction::Gt,
		BinOp::Ge => Instruction::Gte,
	}
}

fn lower_global(g: &GlobalInit) -> GlobalSlot {
	match g {
		GlobalInit::Thunk(f) => GlobalSlot::Pending(f.0),
		GlobalInit::PreEvaluated(p) => GlobalSlot::Evaluated(pre_eval_to_value(p)),
	}
}

fn pre_eval_to_value(p: &PreEval) -> Value {
	match p {
		PreEval::Builtin(tag) => Value::Builtin(Rc::from(tag.as_str())),
		PreEval::Const(c) => const_to_value(c),
		PreEval::MethodDict(items) => {
			Value::MethodDict(Rc::new(items.iter().map(pre_eval_to_value).collect()))
		}
	}
}

fn const_to_value(c: &Const) -> Value {
	match c {
		Const::Unit => Value::Nothing,
		Const::Bool(b) => Value::Bool(*b),
		Const::Int(n) => Value::Int(*n),
		Const::Float(f) => Value::Float(*f),
		Const::Str(s) => Value::String(Rc::new(s.clone())),
		Const::Bytes(b) => Value::Bytes(Rc::new(b.clone())),
		Const::Duration(n) => Value::Duration(*n),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use ir::{Block, Const, Function, GlobalId, IrProgram, Rvalue, Stmt, VarId};
	use std::cell::RefCell;

	// The vertical slice end-to-end through the back half of the pipeline:
	// hand-build the IR for `def main = fun { print "hello, world!" }`, emit it,
	// run it on the VM, and confirm the captured stdout.
	#[test]
	fn emits_and_runs_hello() {
		let print_g = GlobalId(0);
		let main_g = GlobalId(1);

		// F0: fun { print "hello, world!" }
		let f0 = Function {
			name: "fun".into(),
			module: "main".into(),
			params: vec![],
			captures: vec![],
			is_async: false,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(VarId(0), Rvalue::GlobalRef(print_g))),
				Stmt::synthetic(StmtKind::Let(
					VarId(1),
					Rvalue::CallClosure(
						Atom::Var(VarId(0)),
						vec![Atom::Const(Const::Str("hello, world!".into()))],
					),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(VarId(1)))),
			]),
		};
		// F1: main's thunk -> a closure of F0 with no captures.
		let f1 = Function {
			name: "main@thunk".into(),
			module: "main".into(),
			params: vec![],
			captures: vec![],
			is_async: false,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(
					VarId(0),
					Rvalue::MakeClosure(ir::FuncId(0), vec![]),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(VarId(0)))),
			]),
		};
		// F2: entry -> load main, call with the unit arg, return.
		let f2 = Function {
			name: "__entry__".into(),
			module: "".into(),
			params: vec![],
			captures: vec![],
			is_async: false,
			body: Block(vec![
				Stmt::synthetic(StmtKind::Let(VarId(0), Rvalue::GlobalRef(main_g))),
				Stmt::synthetic(StmtKind::Let(
					VarId(1),
					Rvalue::CallClosure(Atom::Var(VarId(0)), vec![Atom::Const(Const::Unit)]),
				)),
				Stmt::synthetic(StmtKind::Return(Atom::Var(VarId(1)))),
			]),
		};

		let program = IrProgram {
			functions: vec![f0, f1, f2],
			globals: vec![
				GlobalInit::PreEvaluated(PreEval::Builtin("print".into())),
				GlobalInit::Thunk(ir::FuncId(1)),
			],
			enums: HashMap::new(),
			entry: ir::FuncId(2),
			test_suites: vec![],
		};

		let vm_program = emit(&program).expect("emit should succeed");
		let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
		let mut vm = vm::VM::new(vm_program).with_stdout(vm::OutputSink::Buffer(buf.clone()));
		assert!(vm.run().is_ok(), "vm run should succeed");

		let out = String::from_utf8_lossy(&buf.borrow()).to_string();
		assert!(
			out.contains("hello, world!"),
			"expected greeting in stdout, got {out:?}"
		);
	}
}
