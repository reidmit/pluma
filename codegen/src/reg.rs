//! IR -> register-VM lowering (M1 — see `notes/REGISTER_VM.md`).
//!
//! The register analogue of `from_ir.rs`. Because the IR is already ANF
//! three-address code, this is a *more faithful* lowering than the stack one:
//! a `Let(v, rv)` computes `rv` directly into `v`'s register, and an operand is
//! just its register. The whole pending-stack apparatus from `from_ir.rs`
//! (`count_uses`/`pending`/`stack_prefix_len`/`emit_operands`/`spill_pending`
//! and the conditional-`StoreLocal` peephole) is **gone** — there is no operand
//! stack to schedule onto.
//!
//! ## Naive allocation (M1)
//!
//! One register per `VarId` (no live-range reuse yet). Params are mapped to
//! registers `0..param_count` so the calling convention can drop args there;
//! captures and body vars follow, compactly numbered (`FnCtx::regs`). This
//! handles both `lower`-produced functions and synthesized poll fns (whose
//! params are arbitrary fresh `VarId`s). Above the variable registers sits a
//! stack-disciplined scratch region for:
//!   * constants used as operands (a reg-list / arith op needs a register, so a
//!     `Const` operand is loaded into a temp first), and
//!   * intermediate destinations during pattern destructuring.
//! Temps are saved/restored per statement (stack discipline), so a nested
//! statement's temps never clobber an enclosing one's. M2 replaces this with
//! linear-scan over the liveness `ir::cps` already computes.
//!
//! Captures are materialized into their registers by a `LoadCapture` prologue,
//! after which the body reads them as ordinary registers.

use std::collections::HashMap;
use std::rc::Rc;

use compiler::Range;
use ir::{
	Atom, BinOp, Block, Callee, Const, Function as IrFunction, GlobalInit, IrProgram, ListItem,
	ListRest, Pattern, PreEval, RecordRest, Rvalue, Stmt, StmtKind, VarId,
};
use vm::Value;
use vm::program::GlobalSlot;
use vm::reg::{Function, Instruction, Program, Reg, RegListIdx, RegRepr};

/// Lower a complete IR program to a runnable register-VM `Program`.
pub fn emit(program: &IrProgram) -> Result<Program, String> {
	let mut e = Emitter::default();
	e.enums = program.enums.clone();
	e.fn_is_async = program.functions.iter().map(|f| f.is_async).collect();
	e.fn_param_raw = program
		.functions
		.iter()
		.map(|f| {
			// A param is raw only if the callee uses the raw discipline and the
			// param's *post-coercion* repr (`var_reprs`, not the stale `param_reprs`
			// field `lower` stamps) is I64.
			let coerced = uses_raw_registers(f);
			f.params
				.iter()
				.map(|p| coerced && matches!(f.var_reprs.get(p.0 as usize), Some(ir::Repr::I64)))
				.collect()
		})
		.collect();
	for func in &program.functions {
		let f = e.lower_function(func)?;
		e.functions.push(f);
	}
	if std::env::var("PLUMA_DUMP_REG").is_ok() {
		for (i, f) in e.functions.iter().enumerate() {
			eprintln!(
				"# fn {i} {} params={} nregs={} captures={}",
				f.name, f.param_count, f.nregs, f.capture_count
			);
			for (j, instr) in f.body.iter().enumerate() {
				eprintln!("  {j:3}  {instr:?}");
			}
		}
		for (i, rl) in e.reg_lists.iter().enumerate() {
			eprintln!("# reg_list {i} = {rl:?}");
		}
	}
	let globals = program.globals.iter().map(lower_global).collect();
	// Does any function use the raw (unboxed-`I64`) register discipline? Only then
	// does the VM need its parallel raw window. This is exactly the per-function
	// `coerced` predicate codegen used above — so it's true iff some function emits a
	// raw opcode (incl. raw *temps*, which aren't in `reg_reprs`). With M5/M6 dormant
	// it's `false`, and the VM skips all raw-window maintenance (the per-call resize).
	let uses_raw = program.functions.iter().any(uses_raw_registers);
	Ok(Program {
		functions: e.functions,
		constants: e.constants,
		bytes_constants: e.bytes_constants,
		globals,
		field_lists: e.field_lists,
		reg_lists: e.reg_lists,
		global_by_name: HashMap::new(),
		enum_variants: program.enums.clone(),
		entry: program.entry.0,
		test_suites: program
			.test_suites
			.iter()
			.map(|(m, g)| (m.clone(), g.0))
			.collect(),
		test_new: program.test_new.map(|g| g.0),
		async_poll: program
			.functions
			.iter()
			.map(|f| f.poll_fn.map(|p| p.0))
			.collect(),
		uses_raw,
	})
}

#[derive(Default)]
struct Emitter {
	functions: Vec<Function>,
	constants: Vec<Rc<String>>,
	const_lookup: HashMap<String, u32>,
	bytes_constants: Vec<Rc<Vec<u8>>>,
	bytes_lookup: HashMap<Vec<u8>, u32>,
	field_lists: Vec<Vec<u32>>,
	reg_lists: Vec<Vec<Reg>>,
	enums: HashMap<String, Vec<(String, usize)>>,
	fn_is_async: Vec<bool>,
	/// Per `FuncId`, whether each parameter is an unboxed i64 — so a `CallDirect`
	/// to it materializes a *const* int arg into the raw window (a var arg is
	/// already in the right window via coercion). M6.
	fn_param_raw: Vec<Vec<bool>>,
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

	fn intern_reg_list(&mut self, regs: Vec<Reg>) -> RegListIdx {
		let idx = self.reg_lists.len() as u32;
		self.reg_lists.push(regs);
		idx
	}

	fn lower_function(&mut self, f: &IrFunction) -> Result<Function, String> {
		let mut ctx = FnCtx::new(f);
		let mut body = Vec::new();
		let mut ranges = Vec::new();
		// Prologue: materialize each capture into its register.
		for (i, c) in f.captures.iter().enumerate() {
			push(
				&mut body,
				&mut ranges,
				Instruction::LoadCapture {
					dst: ctx.r(*c),
					idx: i as u16,
				},
				Range::collapsed(0, 0),
			);
		}
		ctx.lower_block(self, &f.body, &mut body, &mut ranges)?;
		let nregs = ctx.peak;
		// Per-register reprs: variable registers from `var_reprs`, temps boxed.
		// (Metadata for the future typed register file; the VM's raw/boxed choice
		// is encoded in the opcodes themselves.)
		let mut reg_reprs = vec![RegRepr::Boxed; nregs as usize];
		reg_reprs[..ctx.var_reprs.len()].copy_from_slice(&ctx.var_reprs);
		Ok(Function {
			name: f.name.clone(),
			module: f.module.clone(),
			param_count: f.params.len() as u16,
			nregs,
			capture_count: f.captures.len() as u16,
			reg_reprs,
			body,
			source_ranges: ranges,
		})
	}
}

struct LoopFrame {
	start: u32,
	breaks: Vec<u32>,
}

/// Per-function lowering context: the `VarId -> Reg` map plus the scratch-temp
/// watermark and active loops.
struct FnCtx {
	/// `VarId.0` -> register. Params occupy `0..param_count` (the calling
	/// convention places call args there); captures and body vars follow,
	/// compactly numbered. Built from the actual `VarId`s so it works both for
	/// `lower`-produced functions (params are already `0..n`) and for synthesized
	/// poll fns, whose `state`/`resume` params are arbitrary fresh `VarId`s.
	regs: HashMap<u32, Reg>,
	/// Per variable register (`0..base`), its repr. `I64` registers live in the
	/// VM's raw window; everything else is boxed. Empty/all-`Boxed` for an
	/// un-coerced function. (Temps aren't tracked here — their repr is known at
	/// the point they're created and consumed.)
	var_reprs: Vec<RegRepr>,
	/// True iff this function uses the raw register discipline (`uses_raw_registers`),
	/// so unboxed (`I64`) registers + raw-variant opcodes are in play. M5/M6.
	coerced: bool,
	/// Next free scratch register (bump allocator, stack-disciplined). Starts
	/// just above the last mapped variable register.
	next_temp: Reg,
	/// High-water mark of `next_temp` = the function's register count.
	peak: u16,
	loops: Vec<LoopFrame>,
}

impl FnCtx {
	fn new(f: &IrFunction) -> Self {
		let mut regs: HashMap<u32, Reg> = HashMap::new();
		// Params first → registers `0..param_count`.
		for (i, p) in f.params.iter().enumerate() {
			regs.insert(p.0, i as Reg);
		}
		// Captures + every body var, in sorted order for determinism.
		let mut all: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
		for c in &f.captures {
			all.insert(c.0);
		}
		collect_block(&f.body, &mut all);
		let mut next = f.params.len() as Reg;
		for v in all {
			if !regs.contains_key(&v) {
				regs.insert(v, next);
				next += 1;
			}
		}
		// A coerced function (uses the raw discipline) gets per-register reprs from
		// `var_reprs`; only `I64` is unboxed in this scope (F64/I32/Boxed → boxed).
		let coerced = uses_raw_registers(f);
		let mut var_reprs = vec![RegRepr::Boxed; next as usize];
		if coerced {
			for (vid, &reg) in &regs {
				if matches!(f.var_reprs.get(*vid as usize), Some(ir::Repr::I64)) {
					var_reprs[reg as usize] = RegRepr::I64;
				}
			}
		}
		FnCtx {
			regs,
			var_reprs,
			coerced,
			next_temp: next,
			peak: next,
			loops: Vec::new(),
		}
	}

	/// The register holding `VarId v`.
	fn r(&self, v: VarId) -> Reg {
		*self
			.regs
			.get(&v.0)
			.unwrap_or_else(|| panic!("reg: unmapped VarId({})", v.0))
	}

	/// The repr of variable register `reg` (`Boxed` for temps / out of range).
	fn reg_repr(&self, reg: Reg) -> RegRepr {
		self
			.var_reprs
			.get(reg as usize)
			.copied()
			.unwrap_or(RegRepr::Boxed)
	}

	fn fresh_temp(&mut self) -> Reg {
		let r = self.next_temp;
		self.next_temp += 1;
		if self.next_temp > self.peak {
			self.peak = self.next_temp;
		}
		r
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
		let r = stmt.range;
		// Stack-disciplined temps: this statement's scratch is freed on exit, so
		// nested statements never clobber an enclosing statement's live temps.
		let save = self.next_temp;
		match &stmt.kind {
			StmtKind::Let(v, rv) => {
				self.lower_rvalue(em, self.r(*v), rv, body, ranges, r)?;
			}
			StmtKind::Discard(rv) => {
				let t = self.fresh_temp();
				self.lower_rvalue(em, t, rv, body, ranges, r)?;
			}
			StmtKind::Return(atom) => {
				// A raw i64 return when the returned value is unboxed i64 (derived
				// from the post-coercion repr, not the stale `ret_repr` field). Under
				// uniform Sigs returns are boxed; raw returns are an M6 (mono) concern.
				// A bare `Const::Int` survives coercion in the `Return` only when the
				// function's own `self_ret` is I64 (otherwise `insert_coercions` would
				// have boxed it to a var) — so in a coerced function it's a raw return
				// and must be materialized into the raw window (`atom_reg_raw`).
				let raw = match atom {
					Atom::Var(v) => self.coerced && self.reg_repr(self.r(*v)) == RegRepr::I64,
					Atom::Const(Const::Int(_)) => self.coerced,
					Atom::Const(_) => false,
				};
				let src = if raw {
					self.atom_reg_raw(em, atom, body, ranges, r)
				} else {
					self.atom_reg(em, atom, body, ranges, r)
				};
				push(body, ranges, Instruction::Return { src, raw }, r);
			}
			StmtKind::If(cond, then_b, else_b) => {
				let c = self.atom_reg(em, cond, body, ranges, r);
				let jf = emit_at(
					body,
					ranges,
					Instruction::JumpIfFalse { cond: c, target: 0 },
					r,
				);
				self.lower_block(em, then_b, body, ranges)?;
				let j_end = emit_at(body, ranges, Instruction::Jump { target: 0 }, r);
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
				push(body, ranges, Instruction::Jump { target: start }, r);
				let frame = self.loops.pop().expect("loop frame");
				let end = body.len() as u32;
				for bj in frame.breaks {
					patch(body, bj, end);
				}
			}
			StmtKind::Break => {
				let j = emit_at(body, ranges, Instruction::Jump { target: 0 }, r);
				self
					.loops
					.last_mut()
					.ok_or("reg: break outside loop")?
					.breaks
					.push(j);
			}
			StmtKind::Continue => {
				let start = self.loops.last().ok_or("reg: continue outside loop")?.start;
				push(body, ranges, Instruction::Jump { target: start }, r);
			}
			StmtKind::PushDefer(closure) => {
				let thunk = self.atom_reg(em, closure, body, ranges, r);
				push(body, ranges, Instruction::PushDefer { thunk }, r);
			}
			StmtKind::Match { subject, arms } => {
				let subj = self.atom_reg(em, subject, body, ranges, r);
				let mut end_jumps = Vec::new();
				for arm in arms {
					let fails = self.emit_pattern(em, subj, &arm.pattern, body, ranges, r)?;
					self.lower_block(em, &arm.body, body, ranges)?;
					end_jumps.push(emit_at(body, ranges, Instruction::Jump { target: 0 }, r));
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
			other => return Err(format!("reg: unsupported statement: {other:?}")),
		}
		self.next_temp = save;
		Ok(())
	}

	/// Emit the match test for `pattern` against the value in `subj`. Returns the
	/// indices of fail-jump instructions, all patched to the next arm by the
	/// caller. Destructuring extracts payload fields directly into destination
	/// registers (the bound var's register for a `Bind` sub-pattern, else a fresh
	/// temp the recursion matches). No operand stack ⇒ no cleanup trampolines.
	fn emit_pattern(
		&mut self,
		em: &mut Emitter,
		subj: Reg,
		pattern: &Pattern,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<Vec<u32>, String> {
		match pattern {
			Pattern::Wildcard => Ok(Vec::new()),
			Pattern::Bind(v) => {
				if self.r(*v) != subj {
					push(
						body,
						ranges,
						Instruction::Move {
							dst: self.r(*v),
							src: subj,
						},
						r,
					);
				}
				Ok(Vec::new())
			}
			Pattern::Literal(c) => {
				let jmp = match c {
					Const::Int(n) => emit_at(
						body,
						ranges,
						Instruction::MatchInt {
							subject: subj,
							val: *n,
							on_fail: 0,
						},
						r,
					),
					Const::Bool(b) => emit_at(
						body,
						ranges,
						Instruction::MatchBool {
							subject: subj,
							val: *b,
							on_fail: 0,
						},
						r,
					),
					Const::Float(f) => emit_at(
						body,
						ranges,
						Instruction::MatchFloat {
							subject: subj,
							val: *f,
							on_fail: 0,
						},
						r,
					),
					Const::Str(s) => {
						let k = em.intern(s);
						emit_at(
							body,
							ranges,
							Instruction::MatchString {
								subject: subj,
								k,
								on_fail: 0,
							},
							r,
						)
					}
					Const::Bytes(b) => {
						let k = em.intern_bytes(b);
						emit_at(
							body,
							ranges,
							Instruction::MatchBytes {
								subject: subj,
								k,
								on_fail: 0,
							},
							r,
						)
					}
					Const::Unit => emit_at(
						body,
						ranges,
						Instruction::MatchNothing {
							subject: subj,
							on_fail: 0,
						},
						r,
					),
					Const::Duration(n) => emit_at(
						body,
						ranges,
						Instruction::MatchDuration {
							subject: subj,
							ns: *n,
							on_fail: 0,
						},
						r,
					),
				};
				Ok(vec![jmp])
			}
			Pattern::Variant { variant, fields } => {
				let dests = self.dests_for(fields);
				let list = em.intern_reg_list(dests.clone());
				let v = em.intern(variant);
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchVariant {
						subject: subj,
						variant: v,
						dests: list,
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				for (i, sub) in fields.iter().enumerate() {
					fails.extend(self.emit_pattern(em, dests[i], sub, body, ranges, r)?);
				}
				Ok(fails)
			}
			Pattern::Tuple(elems) => {
				let dests = self.dests_for(elems);
				let list = em.intern_reg_list(dests.clone());
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchTuple {
						subject: subj,
						dests: list,
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				for (i, sub) in elems.iter().enumerate() {
					fails.extend(self.emit_pattern(em, dests[i], sub, body, ranges, r)?);
				}
				Ok(fails)
			}
			Pattern::List { items, rest } => {
				let mut dests = self.dests_for(items);
				let has_rest = rest.is_some();
				if has_rest {
					dests.push(match rest {
						Some(ListRest::Bind(v)) => self.r(*v),
						_ => self.fresh_temp(),
					});
				}
				let list = em.intern_reg_list(dests.clone());
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchList {
						subject: subj,
						dests: list,
						has_rest,
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				for (i, sub) in items.iter().enumerate() {
					fails.extend(self.emit_pattern(em, dests[i], sub, body, ranges, r)?);
				}
				Ok(fails)
			}
			Pattern::Record { fields, rest, .. } => {
				let names: Vec<u32> = fields.iter().map(|(n, _)| em.intern(n)).collect();
				let fields_idx = em.intern_field_list(names);
				let sub_pats: Vec<&Pattern> = fields.iter().map(|(_, p)| p).collect();
				let mut dests: Vec<Reg> = sub_pats
					.iter()
					.map(|p| match p {
						Pattern::Bind(v) => self.r(*v),
						_ => self.fresh_temp(),
					})
					.collect();
				let with_rest = matches!(rest, RecordRest::Bind(_));
				if let RecordRest::Bind(v) = rest {
					dests.push(self.r(*v));
				}
				let list = em.intern_reg_list(dests.clone());
				let jmp = emit_at(
					body,
					ranges,
					Instruction::MatchRecord {
						subject: subj,
						fields: fields_idx,
						dests: list,
						exact: matches!(rest, RecordRest::Exact),
						with_rest,
						on_fail: 0,
					},
					r,
				);
				let mut fails = vec![jmp];
				for (i, sub) in sub_pats.iter().enumerate() {
					fails.extend(self.emit_pattern(em, dests[i], sub, body, ranges, r)?);
				}
				Ok(fails)
			}
		}
	}

	/// A destination register per sub-pattern: the bound var's register for a
	/// `Bind`, else a fresh temp the recursion will match.
	fn dests_for(&mut self, subs: &[Pattern]) -> Vec<Reg> {
		subs
			.iter()
			.map(|p| match p {
				Pattern::Bind(v) => self.r(*v),
				_ => self.fresh_temp(),
			})
			.collect()
	}

	fn lower_rvalue(
		&mut self,
		em: &mut Emitter,
		dst: Reg,
		rv: &Rvalue,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<(), String> {
		match rv {
			Rvalue::Use(a) => self.emit_into(em, dst, a, body, ranges, r),
			// `Box`: a raw i64 -> boxed `Value::Int`. If the source isn't actually
			// I64 (F64/I32 are boxed in this scope) it's an identity move.
			Rvalue::Box(a) => {
				match a {
					Atom::Var(v) if self.reg_repr(self.r(*v)) == RegRepr::I64 => {
						let src = self.r(*v);
						push(body, ranges, Instruction::Box { dst, src }, r);
					}
					_ => self.emit_into(em, dst, a, body, ranges, r)?,
				}
				Ok(())
			}
			// `Unbox`: a boxed value -> raw i64. Only I64 is unboxed here; F64/I32
			// stay boxed (identity move).
			Rvalue::Unbox(a, repr) => {
				if *repr == ir::Repr::I64 {
					match a {
						Atom::Var(v) => {
							let src = self.r(*v);
							push(body, ranges, Instruction::Unbox { dst, src }, r);
						}
						Atom::Const(Const::Int(n)) => {
							push(body, ranges, Instruction::LoadIntR { dst, val: *n }, r);
						}
						Atom::Const(c) => self.load_const(em, dst, c, body, ranges, r),
					}
				} else {
					self.emit_into(em, dst, a, body, ranges, r)?;
				}
				Ok(())
			}
			Rvalue::Bin(op, a, b) => {
				// In a coerced function the repr pass proved int arithmetic/
				// comparison operands unbox to i64 — emit the raw-window variants.
				if self.coerced && is_raw_int_op(*op) {
					let ra = self.atom_reg_raw(em, a, body, ranges, r);
					let rb = self.atom_reg_raw(em, b, body, ranges, r);
					push(body, ranges, raw_binop_instr(*op, dst, ra, rb), r);
				} else {
					let ra = self.atom_reg(em, a, body, ranges, r);
					let rb = self.atom_reg(em, b, body, ranges, r);
					push(body, ranges, binop_instr(*op, dst, ra, rb), r);
				}
				Ok(())
			}
			Rvalue::Not(a) => {
				let ra = self.atom_reg(em, a, body, ranges, r);
				push(body, ranges, Instruction::LogicalNot { dst, a: ra }, r);
				Ok(())
			}
			Rvalue::GetDictMethod(d, idx) => {
				let dict = self.atom_reg(em, d, body, ranges, r);
				push(
					body,
					ranges,
					Instruction::GetDictField {
						dst,
						dict,
						index: *idx as u16,
					},
					r,
				);
				Ok(())
			}
			Rvalue::MakeDict(methods) => {
				let methods = self.operand_list(em, methods, body, ranges, r);
				push(body, ranges, Instruction::MakeDict { dst, methods }, r);
				Ok(())
			}
			Rvalue::MakeVariant {
				enum_name,
				tag,
				payload,
			} => {
				let (variant_name, _) = em
					.enums
					.get(enum_name)
					.and_then(|vs| vs.get(*tag as usize))
					.ok_or_else(|| format!("reg: unknown variant {enum_name}#{tag}"))?
					.clone();
				let qualified = em.intern(enum_name);
				let variant = em.intern(&variant_name);
				let payload = self.operand_list(em, payload, body, ranges, r);
				push(
					body,
					ranges,
					Instruction::MakeVariant {
						dst,
						qualified,
						variant,
						payload,
					},
					r,
				);
				Ok(())
			}
			Rvalue::MakeVariantCtor { enum_name, tag } => {
				let (variant_name, arity) = em
					.enums
					.get(enum_name)
					.and_then(|vs| vs.get(*tag as usize))
					.ok_or_else(|| format!("reg: unknown variant {enum_name}#{tag}"))?
					.clone();
				let qualified = em.intern(enum_name);
				let variant = em.intern(&variant_name);
				push(
					body,
					ranges,
					Instruction::MakeVariantCtor {
						dst,
						qualified,
						variant,
						arity: arity as u16,
					},
					r,
				);
				Ok(())
			}
			Rvalue::GlobalRef(g) => {
				push(body, ranges, Instruction::LoadGlobal { dst, idx: g.0 }, r);
				Ok(())
			}
			Rvalue::MakeClosure(fid, caps) => {
				let captures = self.operand_list(em, caps, body, ranges, r);
				let fn_idx = fid.0;
				let instr = if em
					.fn_is_async
					.get(fn_idx as usize)
					.copied()
					.unwrap_or(false)
				{
					Instruction::MakeAsyncClosure {
						dst,
						fn_idx,
						captures,
					}
				} else {
					Instruction::MakeClosure {
						dst,
						fn_idx,
						captures,
					}
				};
				push(body, ranges, instr, r);
				Ok(())
			}
			Rvalue::CallClosure(callee, args) => {
				let callee = self.atom_reg(em, callee, body, ranges, r);
				let args = self.operand_list(em, args, body, ranges, r);
				push(body, ranges, Instruction::Call { dst, callee, args }, r);
				Ok(())
			}
			Rvalue::TailCall(callee, args) => {
				let callee = self.atom_reg(em, callee, body, ranges, r);
				let args = self.operand_list(em, args, body, ranges, r);
				// `dst` matters only for a non-closure callee (ctor/builtin/async),
				// where there's no frame to reuse: the value is written to `dst` and
				// the following `Return(dst)` returns it. For a closure callee the
				// frame is reused and `dst` is ignored.
				push(body, ranges, Instruction::TailCall { dst, callee, args }, r);
				Ok(())
			}
			Rvalue::Call(callee, args) => match callee {
				Callee::Function(f) => {
					// Materialize each arg in the callee's param repr: a const int
					// arg to an i64 param goes in the raw window (a var arg is already
					// there via coercion). Other args are boxed.
					let regs: Vec<Reg> = args
						.iter()
						.enumerate()
						.map(|(i, a)| {
							if em
								.fn_param_raw
								.get(f.0 as usize)
								.and_then(|ps| ps.get(i))
								.copied()
								== Some(true)
							{
								self.atom_reg_raw(em, a, body, ranges, r)
							} else {
								self.atom_reg(em, a, body, ranges, r)
							}
						})
						.collect();
					let args = em.intern_reg_list(regs);
					push(
						body,
						ranges,
						Instruction::CallDirect {
							dst,
							fn_idx: f.0,
							args,
						},
						r,
					);
					Ok(())
				}
				Callee::Global(g) => {
					let c = self.fresh_temp();
					push(
						body,
						ranges,
						Instruction::LoadGlobal { dst: c, idx: g.0 },
						r,
					);
					let args = self.operand_list(em, args, body, ranges, r);
					push(
						body,
						ranges,
						Instruction::Call {
							dst,
							callee: c,
							args,
						},
						r,
					);
					Ok(())
				}
				Callee::Builtin(_) => Err("reg: Callee::Builtin not yet supported".to_string()),
			},
			Rvalue::MakeTuple(elems) => {
				let items = self.operand_list(em, elems, body, ranges, r);
				push(body, ranges, Instruction::MakeTuple { dst, items }, r);
				Ok(())
			}
			Rvalue::MakeRecord(fields) => {
				let values: Vec<&Atom> = fields.iter().map(|(_, v)| v).collect();
				let values = self.operand_list_refs(em, &values, body, ranges, r);
				let names: Vec<u32> = fields.iter().map(|(n, _)| em.intern(n)).collect();
				let fields_idx = em.intern_field_list(names);
				push(
					body,
					ranges,
					Instruction::MakeRecord {
						dst,
						values,
						fields: fields_idx,
					},
					r,
				);
				Ok(())
			}
			Rvalue::RecordUpdate { base, fields } => {
				let record = self.atom_reg(em, base, body, ranges, r);
				let vals: Vec<&Atom> = fields.iter().map(|(_, v)| v).collect();
				let values = self.operand_list_refs(em, &vals, body, ranges, r);
				let names: Vec<u32> = fields.iter().map(|(n, _)| em.intern(n)).collect();
				let fields_idx = em.intern_field_list(names);
				push(
					body,
					ranges,
					Instruction::UpdateRecord {
						dst,
						record,
						values,
						fields: fields_idx,
					},
					r,
				);
				Ok(())
			}
			Rvalue::MakeList(items) => {
				if items.iter().any(|i| matches!(i, ListItem::Spread(_))) {
					self.lower_list_spread(em, dst, items, body, ranges, r)
				} else {
					let elems: Vec<&Atom> = items
						.iter()
						.map(|i| match i {
							ListItem::Elem(a) => a,
							ListItem::Spread(_) => unreachable!(),
						})
						.collect();
					let items = self.operand_list_refs(em, &elems, body, ranges, r);
					push(body, ranges, Instruction::MakeList { dst, items }, r);
					Ok(())
				}
			}
			Rvalue::GetField(recv, name, _) => {
				let record = self.atom_reg(em, recv, body, ranges, r);
				let name = em.intern(name);
				push(body, ranges, Instruction::GetField { dst, record, name }, r);
				Ok(())
			}
			Rvalue::GetElement(recv, index) => {
				let tuple = self.atom_reg(em, recv, body, ranges, r);
				push(
					body,
					ranges,
					Instruction::GetElement {
						dst,
						tuple,
						index: *index as u16,
					},
					r,
				);
				Ok(())
			}
			Rvalue::Interpolate(parts) => {
				let parts = self.operand_list(em, parts, body, ranges, r);
				push(body, ranges, Instruction::Interpolate { dst, parts }, r);
				Ok(())
			}
			Rvalue::Await(task) => {
				let task = self.atom_reg(em, task, body, ranges, r);
				push(body, ranges, Instruction::Await { dst, task }, r);
				Ok(())
			}
			other => Err(format!("reg: unsupported rvalue: {other:?}")),
		}
	}

	/// Spread list literal (`[a, ...xs, b]`): each run of plain elements becomes
	/// a `MakeList` segment and each spread its own segment, joined by
	/// `ConcatLists`; a lone `[...xs]` is just `xs`.
	fn lower_list_spread(
		&mut self,
		em: &mut Emitter,
		dst: Reg,
		items: &[ListItem],
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<(), String> {
		if items.len() == 1 {
			if let ListItem::Spread(a) = &items[0] {
				return self.emit_into(em, dst, a, body, ranges, r);
			}
		}
		let mut segments: Vec<Reg> = Vec::new();
		let mut run: Vec<Reg> = Vec::new();
		for i in items {
			match i {
				ListItem::Elem(a) => run.push(self.atom_reg(em, a, body, ranges, r)),
				ListItem::Spread(a) => {
					if !run.is_empty() {
						let items = em.intern_reg_list(std::mem::take(&mut run));
						let t = self.fresh_temp();
						push(body, ranges, Instruction::MakeList { dst: t, items }, r);
						segments.push(t);
					}
					segments.push(self.atom_reg(em, a, body, ranges, r));
				}
			}
		}
		if !run.is_empty() {
			let items = em.intern_reg_list(run);
			let t = self.fresh_temp();
			push(body, ranges, Instruction::MakeList { dst: t, items }, r);
			segments.push(t);
		}
		let lists = em.intern_reg_list(segments);
		push(body, ranges, Instruction::ConcatLists { dst, lists }, r);
		Ok(())
	}

	/// Place an atom into a specific register: a `Move` (elided if already there)
	/// for a var, a direct `Load*` for a constant.
	fn emit_into(
		&mut self,
		em: &mut Emitter,
		dst: Reg,
		atom: &Atom,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Result<(), String> {
		// A copy preserves repr: if the destination is a raw i64 register, use the
		// raw window (`MoveR` / `LoadIntR`); otherwise the boxed path.
		let raw_dst = self.reg_repr(dst) == RegRepr::I64;
		match atom {
			Atom::Var(v) => {
				let src = self.r(*v);
				if src != dst {
					let mv = if raw_dst {
						Instruction::MoveR { dst, src }
					} else {
						Instruction::Move { dst, src }
					};
					push(body, ranges, mv, r);
				}
			}
			Atom::Const(Const::Int(n)) if raw_dst => {
				push(body, ranges, Instruction::LoadIntR { dst, val: *n }, r);
			}
			Atom::Const(c) => self.load_const(em, dst, c, body, ranges, r),
		}
		Ok(())
	}

	/// The register holding `atom` as a boxed value: its own register for a var,
	/// or a fresh temp loaded with the constant (boxed).
	fn atom_reg(
		&mut self,
		em: &mut Emitter,
		atom: &Atom,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Reg {
		match atom {
			Atom::Var(v) => self.r(*v),
			Atom::Const(c) => {
				let t = self.fresh_temp();
				self.load_const(em, t, c, body, ranges, r);
				t
			}
		}
	}

	/// The register holding `atom` as a raw i64 (for an unboxed-int operand): the
	/// var's register (already i64 via coercion), or a fresh temp loaded with
	/// `LoadIntR`. Non-int consts fall back to a boxed load (never expected here).
	fn atom_reg_raw(
		&mut self,
		em: &mut Emitter,
		atom: &Atom,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> Reg {
		match atom {
			Atom::Var(v) => self.r(*v),
			Atom::Const(Const::Int(n)) => {
				let t = self.fresh_temp();
				push(body, ranges, Instruction::LoadIntR { dst: t, val: *n }, r);
				t
			}
			Atom::Const(c) => {
				let t = self.fresh_temp();
				self.load_const(em, t, c, body, ranges, r);
				t
			}
		}
	}

	fn load_const(
		&mut self,
		em: &mut Emitter,
		dst: Reg,
		c: &Const,
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) {
		let instr = match c {
			Const::Unit => Instruction::LoadNothing { dst },
			Const::Bool(b) => Instruction::LoadBool { dst, val: *b },
			Const::Int(n) => Instruction::LoadInt { dst, val: *n },
			Const::Float(f) => Instruction::LoadFloat { dst, val: *f },
			Const::Str(s) => Instruction::LoadConst {
				dst,
				k: em.intern(s),
			},
			Const::Bytes(b) => Instruction::LoadBytes {
				dst,
				k: em.intern_bytes(b),
			},
			Const::Duration(n) => Instruction::LoadDuration { dst, ns: *n },
		};
		push(body, ranges, instr, r);
	}

	fn operand_list(
		&mut self,
		em: &mut Emitter,
		atoms: &[Atom],
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> RegListIdx {
		let refs: Vec<&Atom> = atoms.iter().collect();
		self.operand_list_refs(em, &refs, body, ranges, r)
	}

	fn operand_list_refs(
		&mut self,
		em: &mut Emitter,
		atoms: &[&Atom],
		body: &mut Vec<Instruction>,
		ranges: &mut Vec<Range>,
		r: Range,
	) -> RegListIdx {
		let regs: Vec<Reg> = atoms
			.iter()
			.map(|a| self.atom_reg(em, a, body, ranges, r))
			.collect();
		em.intern_reg_list(regs)
	}
}

fn push(body: &mut Vec<Instruction>, ranges: &mut Vec<Range>, instr: Instruction, range: Range) {
	body.push(instr);
	ranges.push(range);
}

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
	use Instruction::*;
	match &mut body[idx as usize] {
		Jump { target: o }
		| JumpIfFalse { target: o, .. }
		| MatchInt { on_fail: o, .. }
		| MatchFloat { on_fail: o, .. }
		| MatchDuration { on_fail: o, .. }
		| MatchString { on_fail: o, .. }
		| MatchBytes { on_fail: o, .. }
		| MatchBool { on_fail: o, .. }
		| MatchNothing { on_fail: o, .. }
		| MatchVariant { on_fail: o, .. }
		| MatchTuple { on_fail: o, .. }
		| MatchList { on_fail: o, .. }
		| MatchRecord { on_fail: o, .. } => *o = target,
		other => panic!("reg patch: not a jump-like instruction: {other:?}"),
	}
}

fn binop_instr(op: BinOp, dst: Reg, a: Reg, b: Reg) -> Instruction {
	use Instruction as I;
	match op {
		BinOp::AddInt => I::AddInt { dst, a, b },
		BinOp::SubInt => I::SubInt { dst, a, b },
		BinOp::MulInt => I::MulInt { dst, a, b },
		BinOp::DivInt => I::DivInt { dst, a, b },
		BinOp::RemInt => I::RemInt { dst, a, b },
		BinOp::AddFloat => I::AddFloat { dst, a, b },
		BinOp::SubFloat => I::SubFloat { dst, a, b },
		BinOp::MulFloat => I::MulFloat { dst, a, b },
		BinOp::DivFloat => I::DivFloat { dst, a, b },
		BinOp::RemFloat => I::RemFloat { dst, a, b },
		BinOp::Concat => I::ConcatString { dst, a, b },
		BinOp::And => I::LogicalAnd { dst, a, b },
		BinOp::Or => I::LogicalOr { dst, a, b },
		BinOp::Eq => I::Eq { dst, a, b },
		BinOp::Ne => I::Neq { dst, a, b },
		BinOp::LtI64 => I::LtInt { dst, a, b },
		BinOp::LtF64 => I::LtFloat { dst, a, b },
		BinOp::LeI64 => I::LteInt { dst, a, b },
		BinOp::LeF64 => I::LteFloat { dst, a, b },
		BinOp::GtI64 => I::GtInt { dst, a, b },
		BinOp::GtF64 => I::GtFloat { dst, a, b },
		BinOp::GeI64 => I::GteInt { dst, a, b },
		BinOp::GeF64 => I::GteFloat { dst, a, b },
	}
}

/// The int operators whose operands unbox to a raw i64 (M5). Float ops, `++`,
/// `&&`/`||`, and structural `==`/`!=` are excluded (their operands stay boxed).
fn is_raw_int_op(op: BinOp) -> bool {
	matches!(
		op,
		BinOp::AddInt
			| BinOp::SubInt
			| BinOp::MulInt
			| BinOp::DivInt
			| BinOp::RemInt
			| BinOp::LtI64
			| BinOp::LeI64
			| BinOp::GtI64
			| BinOp::GeI64
	)
}

/// The raw-window variant of an int operator. Arithmetic writes the raw i64 dst;
/// comparisons read raw i64 and write a boxed `Value::Bool`.
fn raw_binop_instr(op: BinOp, dst: Reg, a: Reg, b: Reg) -> Instruction {
	use Instruction as I;
	match op {
		BinOp::AddInt => I::AddIntR { dst, a, b },
		BinOp::SubInt => I::SubIntR { dst, a, b },
		BinOp::MulInt => I::MulIntR { dst, a, b },
		BinOp::DivInt => I::DivIntR { dst, a, b },
		BinOp::RemInt => I::RemIntR { dst, a, b },
		BinOp::LtI64 => I::LtIntR { dst, a, b },
		BinOp::LeI64 => I::LteIntR { dst, a, b },
		BinOp::GtI64 => I::GtIntR { dst, a, b },
		BinOp::GeI64 => I::GteIntR { dst, a, b },
		_ => unreachable!("raw_binop_instr: not a raw int op: {op:?}"),
	}
}

// --- global initializers (identical to the stack lowering) ------------------

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

// --- variable collection (for the VarId -> Reg map) ------------------------

// Collect every `VarId.0` that appears in a function body (`Let`/pattern binds
// and atom reads). `FnCtx::new` adds params + captures separately, then maps the
// union to registers. ANF binds before use, so binds alone would suffice;
// scanning reads too is cheap insurance.

type VarSet = std::collections::BTreeSet<u32>;

/// Whether a function body contains any `Box`/`Unbox` rvalue — i.e. the repr
/// coercion pass ran over it. Only then does codegen use per-register reprs +
/// raw-window opcodes; un-coerced IR (the `cps`/`ir_repr` test inputs) stays
/// all-boxed. M5.
fn block_has_coercions(b: &Block) -> bool {
	fn rv_is_coercion(rv: &Rvalue) -> bool {
		matches!(rv, Rvalue::Box(_) | Rvalue::Unbox(_, _))
	}
	fn walk(b: &Block) -> bool {
		b.0.iter().any(|s| match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => rv_is_coercion(rv),
			StmtKind::If(_, t, e) => walk(t) || walk(e),
			StmtKind::Switch { arms, default, .. } => {
				arms.iter().any(|(_, blk)| walk(blk)) || walk(default)
			}
			StmtKind::Match { arms, .. } => arms.iter().any(|a| walk(&a.body)),
			StmtKind::Loop(blk) => walk(blk),
			_ => false,
		})
	}
	walk(b)
}

/// Whether `f` uses the raw (unboxed-`I64`) register discipline — i.e.
/// `insert_coercions` ran on it under mono signatures. True when *either*:
///   * its body carries `Box`/`Unbox` boundary nodes (`block_has_coercions`), or
///   * a parameter carries an `I64` repr in `var_reprs`.
///
/// The second clause catches the *fully-monomorphic* function — e.g. `fib`,
/// whose `i64` values never cross a boxed boundary, so coercion inserts **zero**
/// `Box`/`Unbox` nodes — which must still receive/return raw `i64` to match what
/// its (monomorphized) callers pass and read. The two clauses are exhaustive: any
/// `i64` value in a coerced function either rides an `i64` param or crosses a
/// boxed boundary. And there is no false positive: uniform lowering never seeds a
/// param repr (params stay `Boxed` in `var_reprs`), so an `I64` param repr can
/// only come from mono-mode `infer_reprs` seeding.
fn uses_raw_registers(f: &IrFunction) -> bool {
	block_has_coercions(&f.body)
		|| f
			.params
			.iter()
			.any(|p| matches!(f.var_reprs.get(p.0 as usize), Some(ir::Repr::I64)))
}

fn collect_block(b: &Block, set: &mut VarSet) {
	for s in &b.0 {
		collect_stmt(&s.kind, set);
	}
}

fn collect_stmt(s: &StmtKind, set: &mut VarSet) {
	match s {
		StmtKind::Let(v, rv) => {
			set.insert(v.0);
			collect_rvalue(rv, set);
		}
		StmtKind::Discard(rv) => collect_rvalue(rv, set),
		StmtKind::Return(a) | StmtKind::PushDefer(a) => collect_atom(a, set),
		StmtKind::If(c, t, e) => {
			collect_atom(c, set);
			collect_block(t, set);
			collect_block(e, set);
		}
		StmtKind::Switch {
			scrutinee,
			arms,
			default,
		} => {
			collect_atom(scrutinee, set);
			for (_, b) in arms {
				collect_block(b, set);
			}
			collect_block(default, set);
		}
		StmtKind::Match { subject, arms } => {
			collect_atom(subject, set);
			for arm in arms {
				collect_pattern(&arm.pattern, set);
				collect_block(&arm.body, set);
			}
		}
		StmtKind::Loop(b) => collect_block(b, set),
		StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
	}
}

fn collect_pattern(p: &Pattern, set: &mut VarSet) {
	match p {
		Pattern::Bind(v) => {
			set.insert(v.0);
		}
		Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
			for f in fields {
				collect_pattern(f, set);
			}
		}
		Pattern::List { items, rest } => {
			for i in items {
				collect_pattern(i, set);
			}
			if let Some(ListRest::Bind(v)) = rest {
				set.insert(v.0);
			}
		}
		Pattern::Record { fields, rest, .. } => {
			for (_, p) in fields {
				collect_pattern(p, set);
			}
			if let RecordRest::Bind(v) = rest {
				set.insert(v.0);
			}
		}
		Pattern::Wildcard | Pattern::Literal(_) => {}
	}
}

fn collect_atom(a: &Atom, set: &mut VarSet) {
	if let Atom::Var(v) = a {
		set.insert(v.0);
	}
}

fn collect_rvalue(rv: &Rvalue, set: &mut VarSet) {
	match rv {
		Rvalue::Use(a)
		| Rvalue::Not(a)
		| Rvalue::Box(a)
		| Rvalue::Unbox(a, _)
		| Rvalue::GetDictMethod(a, _)
		| Rvalue::GetField(a, _, _)
		| Rvalue::GetElement(a, _)
		| Rvalue::Await(a)
		| Rvalue::GetTag(a)
		| Rvalue::GetPayload(a, _) => collect_atom(a, set),
		Rvalue::Bin(_, a, b) => {
			collect_atom(a, set);
			collect_atom(b, set);
		}
		Rvalue::Call(_, args) => args.iter().for_each(|a| collect_atom(a, set)),
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			collect_atom(c, set);
			args.iter().for_each(|a| collect_atom(a, set));
		}
		Rvalue::MakeDict(xs)
		| Rvalue::MakeTuple(xs)
		| Rvalue::MakeClosure(_, xs)
		| Rvalue::Interpolate(xs)
		| Rvalue::MakeVariant { payload: xs, .. } => xs.iter().for_each(|a| collect_atom(a, set)),
		Rvalue::MakeRecord(fields) => fields.iter().for_each(|(_, a)| collect_atom(a, set)),
		Rvalue::RecordUpdate { base, fields } => {
			collect_atom(base, set);
			fields.iter().for_each(|(_, a)| collect_atom(a, set));
		}
		Rvalue::MakeList(items) => items.iter().for_each(|it| match it {
			ListItem::Elem(a) | ListItem::Spread(a) => collect_atom(a, set),
		}),
		Rvalue::GlobalRef(_) | Rvalue::MakeVariantCtor { .. } | Rvalue::Builtin(_) => {}
	}
}
