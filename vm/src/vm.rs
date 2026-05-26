// The VM dispatch loop.

use crate::builtin;
use crate::instruction::Instruction;
use crate::program::{Function, GlobalSlot, Program};
use crate::value::{values_eq, ClosureData, Value, VariantCtorData, VariantData};
use compiler::Range;
use std::cell::RefCell;
use std::rc::Rc;

pub struct RuntimeError {
	pub message: String,
	pub range: Option<Range>,
	// Fully-qualified module name where the error was raised. Set alongside
	// `range` by the dispatcher when a builtin returns an error. The CLI uses
	// it to resolve a source path for the failure (paired with `range` to
	// render `file:line:col`).
	pub module: Option<String>,
	// True when this is a deliberate, program-controlled abort (`io.fail`,
	// `result.expect`, `option.expect`) rather than an internal VM fault. The
	// CLI prints the bare message for these instead of the `Runtime error:`
	// prefix it reserves for genuine VM bugs.
	pub is_user_abort: bool,
}

impl RuntimeError {
	pub fn new(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			range: None,
			module: None,
			is_user_abort: false,
		}
	}
	// A program-controlled abort: a clean message + nonzero exit, the engine
	// behind `io.fail` and `expect`. Distinct from `new` so the CLI can tell
	// an intended bail from a VM fault.
	pub fn user_abort(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			range: None,
			module: None,
			is_user_abort: true,
		}
	}
	pub fn at(mut self, range: Range) -> Self {
		self.range = Some(range);
		self
	}
	pub fn in_module(mut self, module: impl Into<String>) -> Self {
		self.module = Some(module.into());
		self
	}
}

pub enum InputSource {
	Stdin,
	// Bytes that haven't been read yet; reads drain from the front.
	Buffer(Rc<RefCell<Vec<u8>>>),
}

impl InputSource {
	// Read up to and including the next '\n'. Returns the line *without*
	// the trailing newline (and without a preceding '\r' if it was \r\n).
	// Returns Ok(None) on EOF.
	pub fn read_line(&self) -> std::io::Result<Option<String>> {
		match self {
			InputSource::Stdin => {
				let mut buf = String::new();
				let n = std::io::stdin().read_line(&mut buf)?;
				if n == 0 {
					return Ok(None);
				}
				if buf.ends_with('\n') {
					buf.pop();
				}
				if buf.ends_with('\r') {
					buf.pop();
				}
				Ok(Some(buf))
			}
			InputSource::Buffer(b) => {
				let mut buf = b.borrow_mut();
				if buf.is_empty() {
					return Ok(None);
				}
				let mut line_bytes: Vec<u8> = match buf.iter().position(|&c| c == b'\n') {
					Some(nl) => buf.drain(..=nl).take(nl).collect(),
					None => std::mem::take(&mut *buf),
				};
				if line_bytes.last() == Some(&b'\r') {
					line_bytes.pop();
				}
				Ok(Some(String::from_utf8_lossy(&line_bytes).to_string()))
			}
		}
	}

	pub fn read_all(&self) -> std::io::Result<String> {
		use std::io::Read;
		match self {
			InputSource::Stdin => {
				let mut s = String::new();
				std::io::stdin().read_to_string(&mut s)?;
				Ok(s)
			}
			InputSource::Buffer(b) => {
				let bytes = std::mem::take(&mut *b.borrow_mut());
				Ok(String::from_utf8_lossy(&bytes).to_string())
			}
		}
	}

	// Drain the source as raw bytes. Used by `io.read-all-bytes`: skips the
	// UTF-8 decode that `read_all` does so binary stdin survives intact.
	pub fn read_all_bytes(&self) -> std::io::Result<Vec<u8>> {
		use std::io::Read;
		match self {
			InputSource::Stdin => {
				let mut buf = Vec::new();
				std::io::stdin().read_to_end(&mut buf)?;
				Ok(buf)
			}
			InputSource::Buffer(b) => Ok(std::mem::take(&mut *b.borrow_mut())),
		}
	}
}

pub enum OutputSink {
	Stdout,
	Stderr,
	Buffer(Rc<RefCell<Vec<u8>>>),
}

impl OutputSink {
	pub fn write_line(&self, s: &str) {
		match self {
			OutputSink::Stdout => println!("{}", s),
			OutputSink::Stderr => eprintln!("{}", s),
			OutputSink::Buffer(buf) => {
				let mut b = buf.borrow_mut();
				b.extend_from_slice(s.as_bytes());
				b.push(b'\n');
			}
		}
	}

	pub fn write(&self, s: &str) {
		use std::io::Write;
		match self {
			OutputSink::Stdout => {
				print!("{}", s);
				let _ = std::io::stdout().flush();
			}
			OutputSink::Stderr => {
				eprint!("{}", s);
				let _ = std::io::stderr().flush();
			}
			OutputSink::Buffer(buf) => {
				buf.borrow_mut().extend_from_slice(s.as_bytes());
			}
		}
	}

	// Raw byte write — no Display formatting, no trailing newline. Used by
	// `io.write-bytes` / `io.write-err-bytes` so callers can emit binary
	// data without going through UTF-8.
	pub fn write_bytes(&self, b: &[u8]) {
		use std::io::Write;
		match self {
			OutputSink::Stdout => {
				let _ = std::io::stdout().write_all(b);
				let _ = std::io::stdout().flush();
			}
			OutputSink::Stderr => {
				let _ = std::io::stderr().write_all(b);
				let _ = std::io::stderr().flush();
			}
			OutputSink::Buffer(buf) => {
				buf.borrow_mut().extend_from_slice(b);
			}
		}
	}
}

// Frames index into the shared `VM::stack` via `base` rather than carrying
// their own Vec of locals. Saves an allocation per Call. `prev_top` is the
// stack length at the moment this frame's setup began — on Return we
// truncate back to it, which discards both the locals and the slot
// occupied by the callee (which sits at `base - 1` for normal calls). For
// the entry frame and for builtin-invoked frames there's no callee on the
// stack, and `prev_top == base`.
pub(crate) struct Frame {
	pub fn_idx: u32,
	pub ip: usize,
	pub base: usize,
	pub slot_count: u16,
	pub prev_top: usize,
	pub captures: Rc<Vec<Value>>,
	// If this frame is forcing a global, the index to write the result to
	// on Return.
	pub forcing_global: Option<u32>,
}

pub struct VM {
	pub program: Program,
	pub stdout: OutputSink,
	pub stderr: OutputSink,
	pub stdin: InputSource,
	// The program's command-line arguments, in order, with the interpreter
	// and script path already stripped by the CLI. Surfaced through the
	// `io.args` builtin; empty unless seeded via `with_args`.
	pub args: Vec<String>,
	pub(crate) stack: Vec<Value>,
	pub(crate) frames: Vec<Frame>,
	// Opt-in opcode-frequency profiling. Set to Some(empty map) before
	// run() to enable; read back after for a count of each opcode.
	pub profile: Option<std::collections::HashMap<&'static str, u64>>,
}

impl VM {
	pub fn new(program: Program) -> Self {
		Self {
			program,
			stdout: OutputSink::Stdout,
			stderr: OutputSink::Stderr,
			stdin: InputSource::Stdin,
			args: Vec::new(),
			stack: Vec::with_capacity(256),
			frames: Vec::with_capacity(64),
			profile: None,
		}
	}

	pub fn with_stdout(mut self, sink: OutputSink) -> Self {
		self.stdout = sink;
		self
	}

	pub fn with_stderr(mut self, sink: OutputSink) -> Self {
		self.stderr = sink;
		self
	}

	pub fn with_stdin(mut self, source: InputSource) -> Self {
		self.stdin = source;
		self
	}

	pub fn with_args(mut self, args: Vec<String>) -> Self {
		self.args = args;
		self
	}

	pub fn run(&mut self) -> Result<Value, RuntimeError> {
		let entry = self.program.entry;
		self.push_frame_with_args(entry, Rc::new(Vec::new()), Vec::new(), None)?;
		self.run_until_frame_depth(0)?;
		let value = self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM exited with empty stack"))?;
		// `main`'s return value doubles as the program's exit status when it's
		// a `result`: an `err e` aborts with `e` on stderr and a nonzero exit
		// — the same controlled exit `io.fail` produces, so the CLI and test
		// harness (both of which already handle a user-abort error) treat it
		// identically. `ok`, and any non-result return such as `nothing`, is
		// success; the value is otherwise discarded. The check is on the
		// runtime `err` tag, the way the `result` builtins dispatch.
		if let Value::Variant(v) = &value {
			if v.variant.as_str() == "err" && v.payload.len() == 1 {
				return Err(RuntimeError::user_abort(format!("{}", v.payload[0])));
			}
		}
		Ok(value)
	}

	// Invoke a top-level `test` block by its global index. Forces the
	// global (which holds the test's zero-arity closure), then drives
	// the closure to completion. Any `RuntimeError` raised by the body
	// — including assertion failures — bubbles up and is the runner's
	// signal that the test failed.
	pub fn call_test(&mut self, global_idx: u32) -> Result<Value, RuntimeError> {
		self.load_global(global_idx)?;
		let closure_val = self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM: test global produced no value"))?;
		let closure = match closure_val {
			Value::Closure(c) => c,
			_ => return Err(RuntimeError::new("VM: test global is not a closure")),
		};
		let depth = self.frames.len();
		self.push_frame_with_args(
			closure.fn_idx as u32,
			Rc::clone(&closure.captures),
			Vec::new(),
			None,
		)?;
		self.run_until_frame_depth(depth)?;
		self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM: test exited with empty stack"))
	}

	// Push a frame whose args are passed as a Vec (no callee on the stack
	// beforehand). Used by the top-level entry, lazy global thunks, and the
	// builtin-invoked closures path. For dispatch-loop calls, see do_call:
	// it leaves the callee + args on the stack and pushes the frame
	// in-place.
	pub(crate) fn push_frame_with_args(
		&mut self,
		fn_idx: u32,
		captures: Rc<Vec<Value>>,
		args: Vec<Value>,
		forcing_global: Option<u32>,
	) -> Result<(), RuntimeError> {
		let func = &self.program.functions[fn_idx as usize];
		let args = if func.param_count == 0 && args.len() == 1 && matches!(args[0], Value::Nothing) {
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
		let prev_top = self.stack.len();
		let base = prev_top;
		let slot_count = func.slot_count as usize;
		self.stack.extend(args);
		self.stack.resize(base + slot_count, Value::Nothing);
		self.frames.push(Frame {
			fn_idx,
			ip: 0,
			base,
			slot_count: slot_count as u16,
			prev_top,
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

	// Fully-qualified module name of the function on top of the call stack.
	// Empty for the synthetic `__entry__` thunk, which has no source module.
	fn current_module(&self) -> Option<String> {
		let frame = self.frames.last()?;
		let func = &self.program.functions[frame.fn_idx as usize];
		if func.module.is_empty() {
			None
		} else {
			Some(func.module.clone())
		}
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
		// `Instruction` is `Copy`, so reading it out by value here is a
		// trivial register-sized move (no allocator, no refcount bumps).
		// This is the hot path: the dispatch loop fires once per executed
		// instruction.
		let instr = func.body[self.frames[frame_idx].ip];
		self.frames[frame_idx].ip += 1;

		if let Some(p) = &mut self.profile {
			*p.entry(opcode_name(&instr)).or_insert(0) += 1;
		}

		match instr {
			Instruction::Pop => {
				self.stack.pop();
			}
			Instruction::Dup => {
				let top = self
					.stack
					.last()
					.cloned()
					.ok_or_else(|| RuntimeError::new("VM: Dup on empty stack").at(self.current_range()))?;
				self.stack.push(top);
			}
			Instruction::LoadConst(idx) => {
				let s = self.program.constants[idx as usize].clone();
				self.stack.push(Value::String(s));
			}
			Instruction::LoadBytes(idx) => {
				let b = self.program.bytes_constants[idx as usize].clone();
				self.stack.push(Value::Bytes(b));
			}
			Instruction::LoadInt(n) => self.stack.push(Value::Int(n)),
			Instruction::LoadFloat(n) => self.stack.push(Value::Float(n)),
			Instruction::LoadBool(b) => self.stack.push(Value::Bool(b)),
			Instruction::LoadNothing => self.stack.push(Value::Nothing),
			Instruction::LoadLocal(slot) => {
				let v = self.stack[self.frames[frame_idx].base + slot as usize].clone();
				self.stack.push(v);
			}
			Instruction::StoreLocal(slot) => {
				let v = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: StoreLocal on empty stack").at(self.current_range())
				})?;
				let base = self.frames[frame_idx].base;
				self.stack[base + slot as usize] = v;
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
						return Err(RuntimeError::new("VM: JumpIfFalse with non-bool").at(self.current_range()))
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
					captures: Rc::new(captures),
				})));
			}
			Instruction::Call(arity) => self.do_call(arity, false)?,
			Instruction::TailCall(arity) => self.do_call(arity, true)?,
			Instruction::Return => {
				let ret = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: Return with empty stack").at(self.current_range())
				})?;
				let popped = self.frames.pop().unwrap();
				// Drop everything from this frame's setup onward (locals,
				// any unused intermediates, and the callee slot below).
				self.stack.truncate(popped.prev_top);
				if let Some(global_idx) = popped.forcing_global {
					self.program.globals[global_idx as usize] = GlobalSlot::Evaluated(ret.clone());
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
					elems.push(
						self.stack.pop().ok_or_else(|| {
							RuntimeError::new("VM: MakeList underflow").at(self.current_range())
						})?,
					);
				}
				elems.reverse();
				self.stack.push(Value::List(Rc::new(elems)));
			}
			Instruction::ConcatLists(count) => {
				// Pop `count` lists (top is the last segment), then splice them
				// back-to-front so the result preserves source order.
				let mut segments: Vec<Rc<Vec<Value>>> = Vec::with_capacity(count as usize);
				for _ in 0..count {
					match self.stack.pop() {
						Some(Value::List(xs)) => segments.push(xs),
						Some(_) => {
							return Err(RuntimeError::new("ConcatLists: expected lists").at(self.current_range()))
						}
						None => {
							return Err(RuntimeError::new("VM: ConcatLists underflow").at(self.current_range()))
						}
					}
				}
				segments.reverse();
				let total: usize = segments.iter().map(|xs| xs.len()).sum();
				let mut out: Vec<Value> = Vec::with_capacity(total);
				for xs in segments {
					out.extend(xs.iter().cloned());
				}
				self.stack.push(Value::List(Rc::new(out)));
			}
			Instruction::MakeRecord(fields_idx) => {
				// Take the field list by value via clone of the indices. The
				// indices are cheap (u32s) and we avoid borrowing
				// `self.program.field_lists` across stack mutations.
				let len = self.program.field_lists[fields_idx as usize].len();
				let mut map = std::collections::HashMap::with_capacity(len);
				for i in (0..len).rev() {
					let v = self.stack.pop().ok_or_else(|| {
						RuntimeError::new("VM: MakeRecord underflow").at(self.current_range())
					})?;
					let name_idx = self.program.field_lists[fields_idx as usize][i];
					let name = self.program.constants[name_idx as usize].clone();
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
						return Err(
							RuntimeError::new(format!("field access `.{}` on non-record value", name))
								.at(self.current_range()),
						)
					}
				}
			}
			Instruction::GetDictField(idx) => {
				let v = self.stack.pop().ok_or_else(|| {
					RuntimeError::new("VM: GetDictField on empty stack").at(self.current_range())
				})?;
				match v {
					Value::MethodDict(methods) => {
						let m = methods.get(idx as usize).ok_or_else(|| {
							RuntimeError::new(format!(
								"VM: GetDictField index {} out of range (dict size {})",
								idx,
								methods.len()
							))
							.at(self.current_range())
						})?;
						self.stack.push(m.clone());
					}
					_ => {
						return Err(
							RuntimeError::new("VM: GetDictField on non-dict value").at(self.current_range()),
						)
					}
				}
			}
			Instruction::MakeDict(size) => {
				let n = size as usize;
				if self.stack.len() < n {
					return Err(
						RuntimeError::new(format!(
							"VM: MakeDict({}) underflow (stack has {} values)",
							n,
							self.stack.len()
						))
						.at(self.current_range()),
					);
				}
				let start = self.stack.len() - n;
				let methods: Vec<Value> = self.stack.drain(start..).collect();
				self.stack.push(Value::MethodDict(Rc::new(methods)));
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
			Instruction::MatchInt(n, on_fail) => {
				self.match_literal(on_fail, |v| matches!(v, Value::Int(x) if *x == n))?
			}
			Instruction::MatchFloat(n, on_fail) => {
				self.match_literal(on_fail, |v| matches!(v, Value::Float(x) if *x == n))?
			}
			Instruction::MatchString(idx, on_fail) => {
				let needle = self.program.constants[idx as usize].clone();
				self.match_literal(on_fail, |v| match v {
					Value::String(s) => s.as_ref() == needle.as_ref(),
					_ => false,
				})?
			}
			Instruction::MatchBytes(idx, on_fail) => {
				let needle = self.program.bytes_constants[idx as usize].clone();
				self.match_literal(on_fail, |v| match v {
					Value::Bytes(b) => b.as_ref() == needle.as_ref(),
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
			Instruction::MatchRecord {
				fields_idx,
				exact,
				with_rest,
				on_fail,
			} => self.match_record(fields_idx, exact, with_rest, on_fail)?,
			Instruction::MatchList {
				arity,
				has_rest,
				on_fail,
			} => self.match_list(arity, has_rest, on_fail)?,
			// Arithmetic, comparison, and unary ops are inlined here (rather
			// than dispatched through helper functions) so the hot loop
			// avoids a function call + a second match on `instr` per
			// instruction. Mismatched value tags are kept as runtime errors
			// even though the analyzer already type-checks operands —
			// defensive, and the unreachable-branches optimize away in
			// release.
			Instruction::AddInt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Int(a.wrapping_add(b))),
					_ => return Err(RuntimeError::new("AddInt: expected ints").at(self.current_range())),
				}
			}
			Instruction::AddFloat => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Float(a + b)),
					_ => return Err(RuntimeError::new("AddFloat: expected floats").at(self.current_range())),
				}
			}
			Instruction::SubInt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Int(a.wrapping_sub(b))),
					_ => return Err(RuntimeError::new("SubInt: expected ints").at(self.current_range())),
				}
			}
			Instruction::SubFloat => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Float(a - b)),
					_ => return Err(RuntimeError::new("SubFloat: expected floats").at(self.current_range())),
				}
			}
			Instruction::MulInt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Int(a.wrapping_mul(b))),
					_ => return Err(RuntimeError::new("MulInt: expected ints").at(self.current_range())),
				}
			}
			Instruction::MulFloat => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Float(a * b)),
					_ => return Err(RuntimeError::new("MulFloat: expected floats").at(self.current_range())),
				}
			}
			Instruction::DivInt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(_), Value::Int(0)) => {
						return Err(RuntimeError::new("division by zero").at(self.current_range()))
					}
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Int(a / b)),
					_ => return Err(RuntimeError::new("DivInt: expected ints").at(self.current_range())),
				}
			}
			Instruction::DivFloat => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Float(a / b)),
					_ => return Err(RuntimeError::new("DivFloat: expected floats").at(self.current_range())),
				}
			}
			Instruction::RemInt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(_), Value::Int(0)) => {
						return Err(RuntimeError::new("division by zero").at(self.current_range()))
					}
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Int(a % b)),
					_ => return Err(RuntimeError::new("RemInt: expected ints").at(self.current_range())),
				}
			}
			Instruction::RemFloat => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Float(a % b)),
					_ => return Err(RuntimeError::new("RemFloat: expected floats").at(self.current_range())),
				}
			}
			Instruction::NegInt => {
				let v = self.stack.pop().unwrap();
				match v {
					Value::Int(n) => self.stack.push(Value::Int(n.wrapping_neg())),
					_ => return Err(RuntimeError::new("NegInt: expected int").at(self.current_range())),
				}
			}
			Instruction::NegFloat => {
				let v = self.stack.pop().unwrap();
				match v {
					Value::Float(n) => self.stack.push(Value::Float(-n)),
					_ => return Err(RuntimeError::new("NegFloat: expected float").at(self.current_range())),
				}
			}
			Instruction::ConcatString => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::String(a), Value::String(b)) => {
						let mut s = String::with_capacity(a.len() + b.len());
						s.push_str(&a);
						s.push_str(&b);
						self.stack.push(Value::String(Rc::new(s)));
					}
					_ => {
						return Err(
							RuntimeError::new("ConcatString: expected strings").at(self.current_range()),
						)
					}
				}
			}
			Instruction::Lt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Bool(a < b)),
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Bool(a < b)),
					_ => return Err(RuntimeError::new("Lt: expected numbers").at(self.current_range())),
				}
			}
			Instruction::Lte => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Bool(a <= b)),
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Bool(a <= b)),
					_ => return Err(RuntimeError::new("Lte: expected numbers").at(self.current_range())),
				}
			}
			Instruction::Gt => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Bool(a > b)),
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Bool(a > b)),
					_ => return Err(RuntimeError::new("Gt: expected numbers").at(self.current_range())),
				}
			}
			Instruction::Gte => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Int(a), Value::Int(b)) => self.stack.push(Value::Bool(a >= b)),
					(Value::Float(a), Value::Float(b)) => self.stack.push(Value::Bool(a >= b)),
					_ => return Err(RuntimeError::new("Gte: expected numbers").at(self.current_range())),
				}
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
					_ => return Err(RuntimeError::new("expected bools for `&&`").at(self.current_range())),
				}
			}
			Instruction::LogicalOr => {
				let r = self.stack.pop().unwrap();
				let l = self.stack.pop().unwrap();
				match (l, r) {
					(Value::Bool(a), Value::Bool(b)) => self.stack.push(Value::Bool(a || b)),
					_ => return Err(RuntimeError::new("expected bools for `||`").at(self.current_range())),
				}
			}
			Instruction::LogicalNot => {
				let v = self.stack.pop().unwrap();
				match v {
					Value::Bool(b) => self.stack.push(Value::Bool(!b)),
					_ => return Err(RuntimeError::new("expected bool for `!`").at(self.current_range())),
				}
			}
		}
		Ok(())
	}

	fn match_literal<F>(&mut self, on_fail: u32, pred: F) -> Result<(), RuntimeError>
	where
		F: FnOnce(&Value) -> bool,
	{
		let subj = self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM: match on empty stack").at(self.current_range()))?;
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
				if v.variant.as_ref() == variant_name.as_ref() && v.payload.len() == arity as usize =>
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
		let subj = self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM: MatchTuple on empty stack").at(self.current_range()))?;
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

	fn match_record(
		&mut self,
		fields_idx: u32,
		exact: bool,
		with_rest: bool,
		on_fail: u32,
	) -> Result<(), RuntimeError> {
		let subj = self.stack.pop().ok_or_else(|| {
			RuntimeError::new("VM: MatchRecord on empty stack").at(self.current_range())
		})?;
		let len = self.program.field_lists[fields_idx as usize].len();
		match subj {
			Value::Record(record) => {
				if exact && record.len() != len {
					let frame_idx = self.frames.len() - 1;
					self.frames[frame_idx].ip = on_fail as usize;
					return Ok(());
				}
				let mut values = Vec::with_capacity(len);
				let mut matched_names: std::collections::HashSet<&str> =
					std::collections::HashSet::with_capacity(len);
				let mut ok = true;
				for i in 0..len {
					let name_idx = self.program.field_lists[fields_idx as usize][i];
					let name = &self.program.constants[name_idx as usize];
					match record.get(name.as_str()) {
						Some(v) => {
							values.push(v.clone());
							matched_names.insert(name.as_str());
						}
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
					if with_rest {
						// Build the rest: the input record minus the
						// matched fields. Heap-allocates a new HashMap,
						// then wraps in Rc.
						let mut rest: std::collections::HashMap<String, Value> =
							std::collections::HashMap::with_capacity(record.len().saturating_sub(len));
						for (k, v) in record.iter() {
							if !matched_names.contains(k.as_str()) {
								rest.insert(k.clone(), v.clone());
							}
						}
						self.stack.push(Value::Record(std::rc::Rc::new(rest)));
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

	fn match_list(&mut self, arity: u16, has_rest: bool, on_fail: u32) -> Result<(), RuntimeError> {
		let subj = self
			.stack
			.pop()
			.ok_or_else(|| RuntimeError::new("VM: MatchList on empty stack").at(self.current_range()))?;
		let arity = arity as usize;
		match subj {
			Value::List(elems) => {
				let len = elems.len();
				let length_ok = if has_rest { len >= arity } else { len == arity };
				if !length_ok {
					let frame_idx = self.frames.len() - 1;
					self.frames[frame_idx].ip = on_fail as usize;
					return Ok(());
				}
				for i in 0..arity {
					self.stack.push(elems[i].clone());
				}
				if has_rest {
					let tail: Vec<Value> = elems[arity..].to_vec();
					self.stack.push(Value::List(std::rc::Rc::new(tail)));
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
			GlobalSlot::Evaluating => Err(
				RuntimeError::new(format!("cycle detected while evaluating global #{}", idx))
					.at(self.current_range()),
			),
			GlobalSlot::Pending(fn_idx) => {
				let fn_idx = *fn_idx;
				self.program.globals[idx as usize] = GlobalSlot::Evaluating;
				// Push the thunk frame. When it returns, the Return
				// handler writes the value into the global slot AND pushes
				// it onto the stack — which is exactly what LoadGlobal
				// wants. Run nested until the thunk completes.
				let depth = self.frames.len();
				self.push_frame_with_args(fn_idx, Rc::new(Vec::new()), Vec::new(), Some(idx))?;
				self.run_until_frame_depth(depth)?;
				Ok(())
			}
		}
	}

	fn do_call(&mut self, arity: u16, tail: bool) -> Result<(), RuntimeError> {
		// Stack layout coming in: [..., callee, arg0, ..., argN-1].
		// For Closure calls we leave the callee + args in place; the new
		// frame's locals start at the args' position. The callee sits at
		// `prev_top` and gets dropped on Return via truncate(prev_top).
		// For Builtin / VariantCtor we don't push a frame, so we pop the
		// args + callee like before.
		let arity = arity as usize;
		let stack_len = self.stack.len();
		if stack_len < arity + 1 {
			return Err(RuntimeError::new("VM: Call underflow").at(self.current_range()));
		}
		let callee_idx = stack_len - arity - 1;
		// Clone the callee value out of the stack. Keeping the slot
		// occupied (rather than removing it) avoids an O(arity) shift.
		let callee = self.stack[callee_idx].clone();
		match callee {
			Value::Closure(c) => {
				let fn_idx = c.fn_idx as u32;
				let captures = Rc::clone(&c.captures);
				let func = &self.program.functions[fn_idx as usize];
				// Normalize zero-arg-with-Nothing: drop the Nothing arg.
				let mut effective_arity = arity;
				if func.param_count == 0
					&& arity == 1
					&& matches!(self.stack[stack_len - 1], Value::Nothing)
				{
					self.stack.pop();
					effective_arity = 0;
				}
				if effective_arity != func.param_count as usize {
					return Err(
						RuntimeError::new(format!(
							"arity mismatch: expected {} args, got {}",
							func.param_count, effective_arity
						))
						.at(self.current_range()),
					);
				}
				let slot_count = func.slot_count as usize;
				if tail {
					// Replace current frame in-place. Move new args down
					// to the current frame's slot range.
					let curr = self.frames.last().unwrap();
					let prev_top = curr.prev_top;
					let new_base = prev_top + 1;
					let stack_len = self.stack.len();
					// Move args from [stack_len - effective_arity .. stack_len]
					// to [new_base .. new_base + effective_arity]. Source and
					// destination can't overlap in practice because the new
					// args sit above the current frame's locals.
					for i in 0..effective_arity {
						let v = self.stack[stack_len - effective_arity + i].clone();
						self.stack[new_base + i] = v;
					}
					self.stack.truncate(new_base + effective_arity);
					self.stack.resize(new_base + slot_count, Value::Nothing);
					let frame = self.frames.last_mut().unwrap();
					frame.fn_idx = fn_idx;
					frame.ip = 0;
					frame.captures = captures;
					frame.base = new_base;
					frame.slot_count = slot_count as u16;
					// prev_top stays the same.
					Ok(())
				} else {
					// Push a new frame using the callee + args already on
					// the stack. The callee at callee_idx becomes the
					// frame's prev_top; the args become the first locals.
					let base = callee_idx + 1;
					self.stack.resize(base + slot_count, Value::Nothing);
					self.frames.push(Frame {
						fn_idx,
						ip: 0,
						base,
						slot_count: slot_count as u16,
						prev_top: callee_idx,
						captures,
						forcing_global: None,
					});
					Ok(())
				}
			}
			Value::Builtin(b) => {
				// Pop args + callee off the stack and call the handler.
				let args_start = stack_len - arity;
				let args: Vec<Value> = self.stack.drain(args_start..).collect();
				self.stack.pop(); // callee
				let range = self.current_range();
				let module = self.current_module();
				let result = builtin::call_builtin(self, b.as_ref(), args).map_err(|e| {
					let mut e = e.at(range);
					if let Some(m) = module {
						e = e.in_module(m);
					}
					e
				})?;
				self.stack.push(result);
				Ok(())
			}
			Value::VariantCtor(c) => {
				if arity != c.arity {
					return Err(
						RuntimeError::new(format!(
							"variant `{}.{}` takes {} arg(s), got {}",
							c.qualified_enum
								.rsplit_once('.')
								.map(|(_, n)| n)
								.unwrap_or(&c.qualified_enum),
							c.variant,
							c.arity,
							arity
						))
						.at(self.current_range()),
					);
				}
				let args_start = stack_len - arity;
				let payload: Vec<Value> = self.stack.drain(args_start..).collect();
				self.stack.pop(); // callee
				self.stack.push(Value::Variant(Rc::new(VariantData {
					qualified_enum: c.qualified_enum.clone(),
					variant: c.variant.clone(),
					payload,
				})));
				Ok(())
			}
			_ => Err(RuntimeError::new("not callable").at(self.current_range())),
		}
	}
}

// Tiny helpers used by builtin::invoke (so VM internals stay private).
impl VM {
	pub(crate) fn frames_len(&self) -> usize {
		self.frames.len()
	}
	pub(crate) fn pop_stack(&mut self) -> Option<Value> {
		self.stack.pop()
	}
	// (module, 1-indexed line) of the call instruction that dispatched the
	// currently running builtin. Used by `debug` to print a call-site header.
	pub(crate) fn current_call_site(&self) -> (String, usize) {
		if let Some(frame) = self.frames.last() {
			let func = &self.program.functions[frame.fn_idx as usize];
			let ip = frame.ip.saturating_sub(1);
			if ip < func.source_ranges.len() {
				let line = func.source_ranges[ip].start.line + 1;
				return (func.module.clone(), line);
			}
		}
		(String::new(), 0)
	}
}

fn opcode_name(i: &Instruction) -> &'static str {
	use Instruction::*;
	match i {
		Pop => "Pop",
		Dup => "Dup",
		LoadConst(_) => "LoadConst",
		LoadBytes(_) => "LoadBytes",
		LoadInt(_) => "LoadInt",
		LoadFloat(_) => "LoadFloat",
		LoadBool(_) => "LoadBool",
		LoadNothing => "LoadNothing",
		LoadLocal(_) => "LoadLocal",
		StoreLocal(_) => "StoreLocal",
		LoadCapture(_) => "LoadCapture",
		LoadGlobal(_) => "LoadGlobal",
		Jump(_) => "Jump",
		JumpIfFalse(_) => "JumpIfFalse",
		MakeClosure { .. } => "MakeClosure",
		Call(_) => "Call",
		TailCall(_) => "TailCall",
		Return => "Return",
		MakeTuple(_) => "MakeTuple",
		MakeList(_) => "MakeList",
		ConcatLists(_) => "ConcatLists",
		MakeRecord { .. } => "MakeRecord",
		MakeVariant { .. } => "MakeVariant",
		MakeVariantCtor { .. } => "MakeVariantCtor",
		GetField(_) => "GetField",
		GetDictField(_) => "GetDictField",
		MakeDict(_) => "MakeDict",
		LoadRegex(_) => "LoadRegex",
		Interpolate(_) => "Interpolate",
		MatchInt(_, _) => "MatchInt",
		MatchFloat(_, _) => "MatchFloat",
		MatchString(_, _) => "MatchString",
		MatchBytes(_, _) => "MatchBytes",
		MatchBool(_, _) => "MatchBool",
		MatchNothing(_) => "MatchNothing",
		MatchVariant { .. } => "MatchVariant",
		MatchTuple { .. } => "MatchTuple",
		MatchRecord { .. } => "MatchRecord",
		MatchList { .. } => "MatchList",
		AddInt => "AddInt",
		AddFloat => "AddFloat",
		SubInt => "SubInt",
		SubFloat => "SubFloat",
		MulInt => "MulInt",
		MulFloat => "MulFloat",
		DivInt => "DivInt",
		DivFloat => "DivFloat",
		RemInt => "RemInt",
		RemFloat => "RemFloat",
		NegInt => "NegInt",
		NegFloat => "NegFloat",
		ConcatString => "ConcatString",
		Lt => "Lt",
		Lte => "Lte",
		Gt => "Gt",
		Gte => "Gte",
		Eq => "Eq",
		Neq => "Neq",
		LogicalAnd => "LogicalAnd",
		LogicalOr => "LogicalOr",
		LogicalNot => "LogicalNot",
	}
}
