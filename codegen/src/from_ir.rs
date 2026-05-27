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
// ops, dict-method dispatch, variant construction, structured control flow
// (`If`/`Loop`/`Break`/`Continue`) and the pattern `Match`. Unsupported nodes
// return an `Err` rather than emitting wrong bytecode, so gaps surface loudly.

use std::collections::HashMap;
use std::rc::Rc;

use compiler::Range;
use ir::{
	Atom, BinOp, Block, Callee, Const, FieldPat, Function as IrFunction, GlobalInit, IrProgram,
	Pattern, PreEval, Rvalue, Stmt, VarId,
};
use vm::program::GlobalSlot;
use vm::{Function, Instruction, Program, Value};

/// Lower a complete IR program to a runnable `vm::Program`.
///
/// IR `FuncId`s are assumed dense and in `functions` order, so a `FuncId(n)`
/// maps to VM function index `n`; the emitter preserves that order.
pub fn emit(program: &IrProgram) -> Result<Program, String> {
	let mut e = Emitter::default();
	e.enums = program.enums.clone();
	for func in &program.functions {
		let f = e.lower_function(func)?;
		e.functions.push(f);
	}
	let globals = program.globals.iter().map(lower_global).collect();
	Ok(Program {
		functions: e.functions,
		constants: e.constants,
		bytes_constants: e.bytes_constants,
		regex_patterns: Vec::new(),
		globals,
		field_lists: Vec::new(),
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
	// qualified-enum-name -> [(variant_name, arity)], for resolving a
	// `MakeVariant` tag back to the variant name the VM wants.
	enums: HashMap<String, Vec<(String, usize)>>,
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
			match stmt {
				Stmt::Let(v, _) => self.assign_slot(*v),
				Stmt::If(_, t, e) => {
					self.assign_let_slots(t);
					self.assign_let_slots(e);
				}
				Stmt::Switch { arms, default, .. } => {
					for (_, b) in arms {
						self.assign_let_slots(b);
					}
					self.assign_let_slots(default);
				}
				Stmt::Loop(b) => self.assign_let_slots(b),
				Stmt::Match { arms, .. } => {
					for arm in arms {
						match &arm.pattern {
							Pattern::Bind(v) => self.assign_slot(*v),
							Pattern::Variant { fields, .. } => {
								for f in fields {
									if let FieldPat::Bind(v) = f {
										self.assign_slot(*v);
									}
								}
							}
							Pattern::Wildcard | Pattern::Literal(_) => {}
						}
						self.assign_let_slots(&arm.body);
					}
				}
				_ => {}
			}
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
		match stmt {
			Stmt::Let(v, rv) => {
				self.lower_rvalue(em, rv, body, ranges)?;
				let slot = self.local_slot(*v)?;
				push(body, ranges, Instruction::StoreLocal(slot));
			}
			Stmt::Discard(rv) => {
				self.lower_rvalue(em, rv, body, ranges)?;
				push(body, ranges, Instruction::Pop);
			}
			Stmt::Return(atom) => {
				self.lower_atom(em, atom, body, ranges)?;
				push(body, ranges, Instruction::Return);
			}
			Stmt::If(cond, then_b, else_b) => {
				self.lower_atom(em, cond, body, ranges)?;
				let jf = emit_at(body, ranges, Instruction::JumpIfFalse(0));
				self.lower_block(em, then_b, body, ranges)?;
				let j_end = emit_at(body, ranges, Instruction::Jump(0));
				let else_start = body.len() as u32;
				patch(body, jf, else_start);
				self.lower_block(em, else_b, body, ranges)?;
				let end = body.len() as u32;
				patch(body, j_end, end);
			}
			Stmt::Loop(b) => {
				let start = body.len() as u32;
				self.loops.push(LoopFrame {
					start,
					breaks: Vec::new(),
				});
				self.lower_block(em, b, body, ranges)?;
				push(body, ranges, Instruction::Jump(start));
				let frame = self.loops.pop().expect("loop frame");
				let end = body.len() as u32;
				for bj in frame.breaks {
					patch(body, bj, end);
				}
			}
			Stmt::Break => {
				let j = emit_at(body, ranges, Instruction::Jump(0));
				self
					.loops
					.last_mut()
					.ok_or("from_ir: break outside loop")?
					.breaks
					.push(j);
			}
			Stmt::Continue => {
				let start = self
					.loops
					.last()
					.ok_or("from_ir: continue outside loop")?
					.start;
				push(body, ranges, Instruction::Jump(start));
			}
			Stmt::Match { subject, arms } => {
				let mut end_jumps = Vec::new();
				for arm in arms {
					self.lower_atom(em, subject, body, ranges)?;
					let fails = self.emit_pattern(em, &arm.pattern, body, ranges)?;
					self.lower_block(em, &arm.body, body, ranges)?;
					end_jumps.push(emit_at(body, ranges, Instruction::Jump(0)));
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
	) -> Result<Vec<u32>, String> {
		match pattern {
			Pattern::Wildcard => {
				push(body, ranges, Instruction::Pop);
				Ok(Vec::new())
			}
			Pattern::Bind(v) => {
				push(body, ranges, Instruction::StoreLocal(self.local_slot(*v)?));
				Ok(Vec::new())
			}
			Pattern::Literal(c) => {
				let jmp = match c {
					Const::Int(n) => emit_at(body, ranges, Instruction::MatchInt(*n, 0)),
					Const::Bool(b) => emit_at(body, ranges, Instruction::MatchBool(*b, 0)),
					Const::Float(f) => emit_at(body, ranges, Instruction::MatchFloat(*f, 0)),
					Const::Str(s) => {
						let idx = em.intern(s);
						emit_at(body, ranges, Instruction::MatchString(idx, 0))
					}
					Const::Bytes(b) => {
						let idx = em.intern_bytes(b);
						emit_at(body, ranges, Instruction::MatchBytes(idx, 0))
					}
					Const::Unit => emit_at(body, ranges, Instruction::MatchNothing(0)),
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
				);
				// Payload is pushed last-on-top, so bind/pop in reverse.
				for fp in fields.iter().rev() {
					match fp {
						FieldPat::Bind(var) => {
							push(
								body,
								ranges,
								Instruction::StoreLocal(self.local_slot(*var)?),
							);
						}
						FieldPat::Wildcard => push(body, ranges, Instruction::Pop),
					}
				}
				Ok(vec![jmp])
			}
		}
	}

	fn lower_rvalue(
		&self,
		em: &mut Emitter,
		rv: &Rvalue,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
	) -> Result<(), String> {
		match rv {
			Rvalue::Use(a) => self.lower_atom(em, a, body, ranges)?,
			Rvalue::Bin(op, a, b) => {
				self.lower_atom(em, a, body, ranges)?;
				self.lower_atom(em, b, body, ranges)?;
				push(body, ranges, binop_instr(*op));
			}
			Rvalue::Not(a) => {
				self.lower_atom(em, a, body, ranges)?;
				push(body, ranges, Instruction::LogicalNot);
			}
			Rvalue::GetDictMethod(dict, idx) => {
				self.lower_atom(em, dict, body, ranges)?;
				push(body, ranges, Instruction::GetDictField(*idx as u16));
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
					self.lower_atom(em, a, body, ranges)?;
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
				);
			}
			Rvalue::GlobalRef(g) => push(body, ranges, Instruction::LoadGlobal(g.0)),
			Rvalue::MakeClosure(fid, caps) => {
				for c in caps {
					self.lower_atom(em, c, body, ranges)?;
				}
				push(
					body,
					ranges,
					Instruction::MakeClosure {
						fn_idx: fid.0,
						num_captures: caps.len() as u16,
					},
				);
			}
			Rvalue::CallClosure(callee, args) => {
				self.lower_atom(em, callee, body, ranges)?;
				for a in args {
					self.lower_atom(em, a, body, ranges)?;
				}
				push(body, ranges, Instruction::Call(args.len() as u16));
			}
			Rvalue::Call(callee, args) => {
				match callee {
					Callee::Global(g) => push(body, ranges, Instruction::LoadGlobal(g.0)),
					Callee::Function(f) => push(
						body,
						ranges,
						Instruction::MakeClosure {
							fn_idx: f.0,
							num_captures: 0,
						},
					),
					Callee::Builtin(_) => {
						return Err("from_ir: Callee::Builtin not yet supported".to_string())
					}
				}
				for a in args {
					self.lower_atom(em, a, body, ranges)?;
				}
				push(body, ranges, Instruction::Call(args.len() as u16));
			}
			other => return Err(format!("from_ir: unsupported rvalue: {other:?}")),
		}
		Ok(())
	}

	fn lower_atom(
		&self,
		em: &mut Emitter,
		atom: &Atom,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
	) -> Result<(), String> {
		match atom {
			Atom::Var(v) => match self.loc(*v)? {
				Loc::Local(s) => push(body, ranges, Instruction::LoadLocal(*s)),
				Loc::Capture(i) => push(body, ranges, Instruction::LoadCapture(*i)),
			},
			Atom::Const(c) => match c {
				Const::Unit => push(body, ranges, Instruction::LoadNothing),
				Const::Bool(b) => push(body, ranges, Instruction::LoadBool(*b)),
				Const::Int(n) => push(body, ranges, Instruction::LoadInt(*n)),
				Const::Float(f) => push(body, ranges, Instruction::LoadFloat(*f)),
				Const::Str(s) => {
					let idx = em.intern(s);
					push(body, ranges, Instruction::LoadConst(idx));
				}
				Const::Bytes(b) => {
					let idx = em.intern_bytes(b);
					push(body, ranges, Instruction::LoadBytes(idx));
				}
			},
		}
		Ok(())
	}
}

fn push(body: &mut Vec<Instruction>, ranges: &mut Vec<Range>, instr: Instruction) {
	body.push(instr);
	ranges.push(Range::collapsed(0, 0));
}

/// Push an instruction and return its index (for later jump patching).
fn emit_at(body: &mut Vec<Instruction>, ranges: &mut Vec<Range>, instr: Instruction) -> u32 {
	let idx = body.len() as u32;
	push(body, ranges, instr);
	idx
}

/// Patch the target offset of a jump-like instruction.
fn patch(body: &mut [Instruction], idx: u32, target: u32) {
	match &mut body[idx as usize] {
		Instruction::Jump(o)
		| Instruction::JumpIfFalse(o)
		| Instruction::MatchInt(_, o)
		| Instruction::MatchFloat(_, o)
		| Instruction::MatchString(_, o)
		| Instruction::MatchBytes(_, o)
		| Instruction::MatchBool(_, o)
		| Instruction::MatchNothing(o)
		| Instruction::MatchVariant { on_fail: o, .. } => *o = target,
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
				Stmt::Let(VarId(0), Rvalue::GlobalRef(print_g)),
				Stmt::Let(
					VarId(1),
					Rvalue::CallClosure(
						Atom::Var(VarId(0)),
						vec![Atom::Const(Const::Str("hello, world!".into()))],
					),
				),
				Stmt::Return(Atom::Var(VarId(1))),
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
				Stmt::Let(VarId(0), Rvalue::MakeClosure(ir::FuncId(0), vec![])),
				Stmt::Return(Atom::Var(VarId(0))),
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
				Stmt::Let(VarId(0), Rvalue::GlobalRef(main_g)),
				Stmt::Let(
					VarId(1),
					Rvalue::CallClosure(Atom::Var(VarId(0)), vec![Atom::Const(Const::Unit)]),
				),
				Stmt::Return(Atom::Var(VarId(1))),
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
