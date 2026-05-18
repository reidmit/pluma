// The VM dispatch loop.

use crate::eval;
use crate::instruction::Instruction;
use crate::program::{Function, GlobalSlot, Program};
use crate::value::{values_eq, ClosureData, Value, VariantCtorData, VariantData};
use compiler::Range;
use std::cell::RefCell;
use std::rc::Rc;

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

pub(crate) struct Frame {
	pub fn_idx: u32,
	pub ip: usize,
	pub locals: Vec<Value>,
	pub captures: Rc<Vec<Value>>,
	// If this frame is forcing a global, the index to write the result to
	// on Return.
	pub forcing_global: Option<u32>,
}

pub struct VM {
	pub program: Program,
	pub stdout: StdoutSink,
	pub(crate) stack: Vec<Value>,
	pub(crate) frames: Vec<Frame>,
}

impl VM {
	pub fn new(program: Program) -> Self {
		Self {
			program,
			stdout: StdoutSink::Real,
			stack: Vec::with_capacity(256),
			frames: Vec::with_capacity(64),
		}
	}

	pub fn with_stdout(mut self, sink: StdoutSink) -> Self {
		self.stdout = sink;
		self
	}

	pub fn run(&mut self) -> Result<Value, RuntimeError> {
		let entry = self.program.entry;
		self.push_frame(entry, Rc::new(Vec::new()), Vec::new(), None)?;
		self.run_until_frame_depth(0)?;
		self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM exited with empty stack"))
	}

	pub(crate) fn push_frame(
		&mut self,
		fn_idx: u32,
		captures: Rc<Vec<Value>>,
		args: Vec<Value>,
		forcing_global: Option<u32>,
	) -> Result<(), RuntimeError> {
		let func = &self.program.functions[fn_idx as usize];
		let args = if func.param_count == 0
			&& args.len() == 1
			&& matches!(args[0], Value::Nothing)
		{
			Vec::new()
		} else {
			args
		};
		if args.len() != func.param_count as usize {
			return Err(RuntimeError::new(format!(
				"arity mismatch: expected {} args, got {}",
				func.param_count,
				args.len()
			)));
		}
		let mut locals = Vec::with_capacity(func.slot_count as usize);
		locals.extend(args);
		locals.resize(func.slot_count as usize, Value::Nothing);
		self.frames.push(Frame {
			fn_idx,
			ip: 0,
			locals,
			captures,
			forcing_global,
		});
		Ok(())
	}

	fn current_range(&self) -> Range {
		if let Some(frame) = self.frames.last() {
			let func = &self.program.functions[frame.fn_idx as usize];
			let ip = frame.ip.saturating_sub(1);
			if ip < func.source_ranges.len() {
				return func.source_ranges[ip];
			}
		}
		Range::collapsed(0, 0)
	}

	// Run until self.frames.len() == target_depth. Used both for the
	// top-level run and for nested invocation by builtins (map, filter,
	// fold, each).
	pub(crate) fn run_until_frame_depth(&mut self, target_depth: usize) -> Result<(), RuntimeError> {
		while self.frames.len() > target_depth {
			self.step()?;
		}
		Ok(())
	}

	fn step(&mut self) -> Result<(), RuntimeError> {
		let frame_idx = self.frames.len() - 1;
		let func: &Function = &self.program.functions[self.frames[frame_idx].fn_idx as usize];
		if self.frames[frame_idx].ip >= func.body.len() {
			return Err(
				RuntimeError::new("VM: ran past end of function (missing Return?)")
					.at(self.current_range()),
			);
		}
		let instr = func.body[self.frames[frame_idx].ip].clone();
		self.frames[frame_idx].ip += 1;

		match instr {
			Instruction::Pop => {
				self.stack.pop();
			}
			Instruction::Dup => {
				let top = self.stack.last().cloned().ok_or_else(|| {
					RuntimeError::new("VM: Dup on empty stack").at(self.current_range())
				})?;
				self.stack.push(top);
			}
			Instruction::LoadConst(idx) => {
				let s = self.program.constants[idx as usize].clone();
				self.stack.push(Value::String(s));
			}
			Instruction::LoadInt(n) => self.stack.push(Value::Int(n)),
			Instruction::LoadFloat(n) => self.stack.push(Value::Float(n)),
			Instruction::LoadBool(b) => self.stack.push(Value::Bool(b)),
			Instruction::LoadNothing => self.stack.push(Value::Nothing),
			Instruction::LoadLocal(slot) => {
				let v = self.frames[frame_idx].locals[slot as usize].clone();
				self.stack.push(v);
			}
			Instruction::StoreLocal(slot) => {
				let v = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: StoreLocal on empty stack").at(self.current_range())
				})?;
				self.frames[frame_idx].locals[slot as usize] = v;
			}
			Instruction::LoadCapture(idx) => {
				let v = self.frames[frame_idx].captures[idx as usize].clone();
				self.stack.push(v);
			}
			Instruction::LoadGlobal(idx) => {
				self.load_global(idx)?;
			}
			Instruction::Jump(off) => {
				self.frames[frame_idx].ip = off as usize;
			}
			Instruction::JumpIfFalse(off) => {
				let v = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: JumpIfFalse on empty stack").at(self.current_range())
				})?;
				match v {
					Value::Bool(false) => self.frames[frame_idx].ip = off as usize,
					Value::Bool(true) => {}
					_ => {
						return Err(RuntimeError::new("VM: JumpIfFalse with non-bool")
							.at(self.current_range()))
					}
				}
			}
			Instruction::MakeClosure {
				fn_idx,
				num_captures,
			} => {
				let mut captures = Vec::with_capacity(num_captures as usize);
				for _ in 0..num_captures {
					captures.push(self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: MakeClosure underflow").at(self.current_range())
					})?);
				}
				captures.reverse();
				self.stack.push(Value::Closure(Rc::new(ClosureData {
					fn_idx: fn_idx as usize,
					captures,
				})));
			}
			Instruction::Call(arity) => self.do_call(arity, false)?,
			Instruction::TailCall(arity) => self.do_call(arity, true)?,
			Instruction::Return => {
				let ret = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: Return with empty stack").at(self.current_range())
				})?;
				let popped = self.frames.pop().unwrap();
				if let Some(global_idx) = popped.forcing_global {
					self.program.globals[global_idx as usize] =
						GlobalSlot::Evaluated(ret.clone());
				}
				self.stack.push(ret);
			}
			Instruction::MakeTuple(arity) => {
				let mut elems = Vec::with_capacity(arity as usize);
				for _ in 0..arity {
					elems.push(self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: MakeTuple underflow").at(self.current_range())
					})?);
				}
				elems.reverse();
				self.stack.push(Value::Tuple(Rc::new(elems)));
			}
			Instruction::MakeList(arity) => {
				let mut elems = Vec::with_capacity(arity as usize);
				for _ in 0..arity {
					elems.push(self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: MakeList underflow").at(self.current_range())
					})?);
				}
				elems.reverse();
				self.stack.push(Value::List(Rc::new(elems)));
			}
			Instruction::MakeRecord { fields } => {
				let mut map = std::collections::HashMap::with_capacity(fields.len());
				for name_idx in fields.iter().rev() {
					let v = self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: MakeRecord underflow").at(self.current_range())
					})?;
					let name = self.program.constants[*name_idx as usize].clone();
					map.insert((*name).clone(), v);
				}
				self.stack.push(Value::Record(Rc::new(map)));
			}
			Instruction::MakeVariant {
				qualified,
				variant,
				arity,
			} => {
				let mut payload = Vec::with_capacity(arity as usize);
				for _ in 0..arity {
					payload.push(self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: MakeVariant underflow").at(self.current_range())
					})?);
				}
				payload.reverse();
				self.stack.push(Value::Variant(Rc::new(VariantData {
					qualified_enum: self.program.constants[qualified as usize].clone(),
					variant: self.program.constants[variant as usize].clone(),
					payload,
				})));
			}
			Instruction::MakeVariantCtor {
				qualified,
				variant,
				arity,
			} => {
				self.stack.push(Value::VariantCtor(Rc::new(VariantCtorData {
					qualified_enum: self.program.constants[qualified as usize].clone(),
					variant: self.program.constants[variant as usize].clone(),
					arity: arity as usize,
				})));
			}
			Instruction::GetField(name_idx) => {
				let v = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: GetField on empty stack").at(self.current_range())
				})?;
				let name = &self.program.constants[name_idx as usize];
				match v {
					Value::Record(fields) => match fields.get(name.as_str()) {
						Some(v) => self.stack.push(v.clone()),
						None => {
							return Err(
								RuntimeError::new(format!("no field `{}` on record", name))
									.at(self.current_range()),
							)
						}
					},
					_ => {
						return Err(RuntimeError::new(format!(
							"field access `.{}` on non-record value",
							name
						))
						.at(self.current_range()))
					}
				}
			}
			Instruction::LoadRegex(idx) => {
				let r = self.program.regex_patterns[idx as usize].clone();
				self.stack.push(Value::Regex(r));
			}
			Instruction::Interpolate(arity) => {
				let mut parts = Vec::with_capacity(arity as usize);
				for _ in 0..arity {
					parts.push(self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: Interpolate underflow").at(self.current_range())
					})?);
				}
				parts.reverse();
				let mut out = String::new();
				for p in &parts {
					match p {
						Value::String(s) => out.push_str(s),
						other => out.push_str(&format!("{}", other)),
					}
				}
				self.stack.push(Value::String(Rc::new(out)));
			}
			Instruction::MatchInt(n, on_fail) => self.match_literal(
				on_fail,
				|v| matches!(v, Value::Int(x) if *x == n),
			)?,
			Instruction::MatchFloat(n, on_fail) => self.match_literal(
				on_fail,
				|v| matches!(v, Value::Float(x) if *x == n),
			)?,
			Instruction::MatchString(idx, on_fail) => {
				let needle = self.program.constants[idx as usize].clone();
				self.match_literal(on_fail, |v| match v {
					Value::String(s) => s.as_ref() == needle.as_ref(),
					_ => false,
				})?
			}
			Instruction::MatchBool(b, on_fail) => {
				self.match_literal(on_fail, |v| matches!(v, Value::Bool(x) if *x == b))?
			}
			Instruction::MatchNothing(on_fail) => {
				self.match_literal(on_fail, |v| matches!(v, Value::Nothing))?
			}
			Instruction::MatchVariant {
				variant,
				arity,
				on_fail,
			} => self.match_variant(variant, arity, on_fail)?,
			Instruction::MatchTuple { arity, on_fail } => self.match_tuple(arity, on_fail)?,
			Instruction::MatchRecord { fields, on_fail } => {
				self.match_record(&fields, on_fail)?
			}
			Instruction::AddInt
			| Instruction::AddFloat
			| Instruction::SubInt
			| Instruction::SubFloat
			| Instruction::MulInt
			| Instruction::MulFloat
			| Instruction::DivInt
			| Instruction::DivFloat
			| Instruction::RemInt
			| Instruction::RemFloat => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				let out = eval::binary(&instr, l, r).map_err(|e| e.at(self.current_range()))?;
				self.stack.push(out);
			}
			Instruction::NegInt | Instruction::NegFloat => {
				let v = self.stack.pop().unwrap();
				let out = eval::unary(&instr, v).map_err(|e| e.at(self.current_range()))?;
				self.stack.push(out);
			}
			Instruction::Lt | Instruction::Lte | Instruction::Gt | Instruction::Gte => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				let out = eval::compare(&instr, l, r).map_err(|e| e.at(self.current_range()))?;
				self.stack.push(out);
			}
			Instruction::Eq => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				self.stack.push(Value::Bool(values_eq(&l, &r)));
			}
			Instruction::Neq => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				self.stack.push(Value::Bool(!values_eq(&l, &r)));
			}
			Instruction::LogicalAnd => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Bool(a), Value::Bool(b)) => self.stack.push(Value::Bool(a && b)),
					_ => {
						return Err(RuntimeError::new("expected bools for `&&`")
							.at(self.current_range()))
					}
				}
			}
			Instruction::LogicalOr => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Bool(a), Value::Bool(b)) => self.stack.push(Value::Bool(a || b)),
					_ => {
						return Err(RuntimeError::new("expected bools for `||`")
							.at(self.current_range()))
					}
				}
			}
			Instruction::LogicalNot => {
				let v = self.stack.pop().unwrap();
				match v {
					Value::Bool(b) => self.stack.push(Value::Bool(!b)),
					_ => {
						return Err(RuntimeError::new("expected bool for `!`")
							.at(self.current_range()))
					}
				}
			}
			Instruction::CallBuiltin(b, arity) => {
				let mut args = Vec::with_capacity(arity as usize);
				for _ in 0..arity {
					args.push(self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: CallBuiltin underflow").at(self.current_range())
					})?);
				}
				args.reverse();
				let result = eval::call_builtin(self, b, args)
					.map_err(|e| e.at(self.current_range()))?;
				self.stack.push(result);
			}
		}
		Ok(())
	}

	fn match_literal<F>(&mut self, on_fail: u32, pred: F) -> Result<(), RuntimeError>
	where
		F: FnOnce(&Value) -> bool,
	{
		let subj = self.stack.pop().ok_or_else(|| {
			RuntimeError::new("VM: match on empty stack").at(self.current_range())
		})?;
		if !pred(&subj) {
			let frame_idx = self.frames.len() - 1;
			self.frames[frame_idx].ip = on_fail as usize;
		}
		Ok(())
	}

	fn match_variant(
		&mut self,
		variant_idx: u32,
		arity: u16,
		on_fail: u32,
	) -> Result<(), RuntimeError> {
		let subj = self.stack.pop().ok_or_else(|| {
			RuntimeError::new("VM: MatchVariant on empty stack").at(self.current_range())
		})?;
		let variant_name = self.program.constants[variant_idx as usize].clone();
		match subj {
			Value::Variant(v)
				if v.variant.as_ref() == variant_name.as_ref()
					&& v.payload.len() == arity as usize =>
			{
				for elem in v.payload.iter() {
					self.stack.push(elem.clone());
				}
			}
			Value::Bool(true) if variant_name.as_ref() == "true" && arity == 0 => {}
			Value::Bool(false) if variant_name.as_ref() == "false" && arity == 0 => {}
			_ => {
				let frame_idx = self.frames.len() - 1;
				self.frames[frame_idx].ip = on_fail as usize;
			}
		}
		Ok(())
	}

	fn match_tuple(&mut self, arity: u16, on_fail: u32) -> Result<(), RuntimeError> {
		let subj = self.stack.pop().ok_or_else(|| {
			RuntimeError::new("VM: MatchTuple on empty stack").at(self.current_range())
		})?;
		match subj {
			Value::Tuple(elems) if elems.len() == arity as usize => {
				for elem in elems.iter() {
					self.stack.push(elem.clone());
				}
			}
			_ => {
				let frame_idx = self.frames.len() - 1;
				self.frames[frame_idx].ip = on_fail as usize;
			}
		}
		Ok(())
	}

	fn match_record(&mut self, fields: &[u32], on_fail: u32) -> Result<(), RuntimeError> {
		let subj = self.stack.pop().ok_or_else(|| {
			RuntimeError::new("VM: MatchRecord on empty stack").at(self.current_range())
		})?;
		match subj {
			Value::Record(record) => {
				let mut values = Vec::with_capacity(fields.len());
				let mut ok = true;
				for name_idx in fields {
					let name = &self.program.constants[*name_idx as usize];
					match record.get(name.as_str()) {
						Some(v) => values.push(v.clone()),
						None => {
							ok = false;
							break;
						}
					}
				}
				if ok {
					for v in values {
						self.stack.push(v);
					}
				} else {
					let frame_idx = self.frames.len() - 1;
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			_ => {
				let frame_idx = self.frames.len() - 1;
				self.frames[frame_idx].ip = on_fail as usize;
			}
		}
		Ok(())
	}

	fn load_global(&mut self, idx: u32) -> Result<(), RuntimeError> {
		match &self.program.globals[idx as usize] {
			GlobalSlot::Evaluated(v) => {
				self.stack.push(v.clone());
				Ok(())
			}
			GlobalSlot::Evaluating => Err(RuntimeError::new(format!(
				"cycle detected while evaluating global #{}",
				idx
			))
			.at(self.current_range())),
			GlobalSlot::Pending(fn_idx) => {
				let fn_idx = *fn_idx;
				self.program.globals[idx as usize] = GlobalSlot::Evaluating;
				// Push the thunk frame. When it returns, the Return
				// handler writes the value into the global slot AND pushes
				// it onto the stack — which is exactly what LoadGlobal
				// wants. Run nested until the thunk completes.
				let depth = self.frames.len();
				self.push_frame(fn_idx, Rc::new(Vec::new()), Vec::new(), Some(idx))?;
				self.run_until_frame_depth(depth)?;
				Ok(())
			}
		}
	}

	fn do_call(&mut self, arity: u16, tail: bool) -> Result<(), RuntimeError> {
		let mut args = Vec::with_capacity(arity as usize);
		for _ in 0..arity {
			args.push(self.stack.pop().ok_or_else(|| {
				RuntimeError::new("VM: Call underflow").at(self.current_range())
			})?);
		}
		args.reverse();
		let callee = self.stack.pop().ok_or_else(|| {
			RuntimeError::new("VM: Call with empty stack").at(self.current_range())
		})?;
		match callee {
			Value::Closure(c) => {
				let fn_idx = c.fn_idx as u32;
				let captures = Rc::new(c.captures.clone());
				if tail {
					let frame_idx = self.frames.len() - 1;
					let func = &self.program.functions[fn_idx as usize];
					let args = if func.param_count == 0
						&& args.len() == 1
						&& matches!(args[0], Value::Nothing)
					{
						Vec::new()
					} else {
						args
					};
					if args.len() != func.param_count as usize {
						return Err(RuntimeError::new(format!(
							"arity mismatch: expected {} args, got {}",
							func.param_count,
							args.len()
						))
						.at(self.current_range()));
					}
					let slot_count = func.slot_count as usize;
					let frame = &mut self.frames[frame_idx];
					frame.fn_idx = fn_idx;
					frame.ip = 0;
					frame.captures = captures;
					frame.locals.clear();
					frame.locals.extend(args);
					frame.locals.resize(slot_count, Value::Nothing);
					Ok(())
				} else {
					self.push_frame(fn_idx, captures, args, None)
				}
			}
			Value::Builtin(b) => {
				let result =
					eval::call_builtin(self, b, args).map_err(|e| e.at(self.current_range()))?;
				self.stack.push(result);
				Ok(())
			}
			Value::VariantCtor(c) => {
				if args.len() != c.arity {
					return Err(RuntimeError::new(format!(
						"variant `{}.{}` takes {} arg(s), got {}",
						c.qualified_enum
							.rsplit_once('.')
							.map(|(_, n)| n)
							.unwrap_or(&c.qualified_enum),
						c.variant,
						c.arity,
						args.len()
					))
					.at(self.current_range()));
				}
				self.stack.push(Value::Variant(Rc::new(VariantData {
					qualified_enum: c.qualified_enum.clone(),
					variant: c.variant.clone(),
					payload: args,
				})));
				Ok(())
			}
			_ => Err(RuntimeError::new("not callable").at(self.current_range())),
		}
	}
}

// Tiny helpers used by eval::invoke (so VM internals stay private).
impl VM {
	pub(crate) fn frames_len(&self) -> usize {
		self.frames.len()
	}
	pub(crate) fn pop_stack(&mut self) -> Option<Value> {
		self.stack.pop()
	}
}
