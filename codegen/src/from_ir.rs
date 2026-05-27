// IR -> bytecode lowering: the second consumer of `ir::IrProgram` (the bytecode
// VM, via this translation; a future WASM backend would be the third).
//
// Translates the target-independent IR into a `vm::Program`. Because the IR is
// ANF + structured control flow, this is a mechanical pass: each `Rvalue`/
// `Stmt` emits a short, local instruction sequence. Storage is assigned here
// (the VM's `base + slot` locals), not in the IR — `VarId`s become stack slots,
// captures become `LoadCapture` indices.
//
// Phase 1.2, growing alongside `ir::lower`. Currently covers the node subset
// the lowering produces for the vertical slice (literals, atoms, global refs,
// closures, calls, returns); unsupported nodes return an `Err` rather than
// emitting wrong bytecode, so coverage gaps surface loudly.

use std::collections::HashMap;
use std::rc::Rc;

use compiler::Range;
use ir::{
	Atom, Block, Callee, Const, Function as IrFunction, GlobalInit, IrProgram, PreEval, Rvalue, Stmt,
	VarId,
};
use vm::program::GlobalSlot;
use vm::{Function, Instruction, Program, Value};

/// Lower a complete IR program to a runnable `vm::Program`.
///
/// IR `FuncId`s are assumed dense and in `functions` order, so a `FuncId(n)`
/// maps to VM function index `n`; the emitter preserves that order.
pub fn emit(program: &IrProgram) -> Result<Program, String> {
	let mut e = Emitter::default();
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
		let ctx = FnCtx::new(f);
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
		};
		ctx.assign_let_slots(&f.body);
		ctx
	}

	/// Pre-assign a local slot to every `let`-bound `VarId`, descending into
	/// nested blocks. ANF guarantees a var is bound before use, so a single
	/// pre-pass suffices.
	fn assign_let_slots(&mut self, block: &Block) {
		for stmt in &block.0 {
			match stmt {
				Stmt::Let(v, _) => {
					if !self.locs.contains_key(&v.0) {
						let s = self.slot_count;
						self.slot_count += 1;
						self.locs.insert(v.0, Loc::Local(s));
					}
				}
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
		&self,
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
		&self,
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
			other => return Err(format!("from_ir: unsupported statement: {other:?}")),
		}
		Ok(())
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
