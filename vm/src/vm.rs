// The register-VM dispatch loop (M1 cutover — see notes/REGISTER_VM.md).
//
// Each function owns a flat register file living in the unified `stack` window
// `stack[base .. base + nregs]`. Instructions name their source and destination
// registers explicitly (three-address), so the stack VM's LoadLocal/StoreLocal
// shuffle is gone. The invariant `stack.len() == base + nregs` for the top frame
// holds after every instruction: registers ARE the window, with no operand
// region above it.

use crate::builtin;
use crate::program::GlobalSlot;
use crate::reg::{Instruction, Program, Reg, RegListIdx, RegRepr};
use crate::value::{
	AsyncFnData, ClosureData, TaskRepr, Value, VariantCtorData, VariantData, values_eq,
};
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

// A call frame. Its register file is the unified-stack window `stack[base ..
// base + nregs]`. There is no operand region above it, so `Return` truncates the
// stack back to `base`. The frame allocates no Vec of its own (saves an
// allocation per call), only the rare `cleanups` Vec when `defer` runs.
pub(crate) struct Frame {
	pub fn_idx: u32,
	pub ip: usize,
	pub base: usize,
	pub nregs: u16,
	pub captures: Rc<Vec<Value>>,
	// Where to deliver this frame's return value. `Some(abs)` writes it to the
	// caller's register at absolute stack index `abs` (a dispatch-loop call).
	// `None` pushes it onto the stack for an external driver to pop (the entry
	// frame, lazy-global thunks, builtin-invoked closures, async step/poll fns).
	pub ret_dst: Option<usize>,
	// Cleanup thunks scheduled by `defer`, in push order. Run LIFO on Return.
	// A frame with pending cleanups can't be reused in place by a tail call.
	pub cleanups: Vec<Value>,
}

pub struct VM {
	pub program: Program,
	pub stdout: OutputSink,
	pub stderr: OutputSink,
	pub stdin: InputSource,
	// The program's command-line arguments, in order, with the interpreter
	// and script path already stripped by the CLI. Surfaced through `io.args`.
	pub args: Vec<String>,
	pub(crate) stack: Vec<Value>,
	// The raw register window. When live (`uses_raw`), it's parallel to `stack` and
	// kept the same length: an I64-repr register `i` of the top frame holds its bits
	// in `raw[base + i]` (a boxed register's `raw` slot is unused, and vice-versa).
	// When `uses_raw` is false (M5/M6 dormant — the default), it stays empty and is
	// never touched. M5 — see notes/REGISTER_VM.md.
	pub(crate) raw: Vec<u64>,
	// Cached `program.uses_raw`: whether any function has an unboxed (`I64`)
	// register. When `false` (M5/M6 dormant — the default), all `raw`-window
	// maintenance is skipped, so `raw` stays empty and costs nothing per call.
	pub(crate) uses_raw: bool,
	pub(crate) frames: Vec<Frame>,
	// The async scheduler. Empty/idle for synchronous programs; populated by
	// `run_task` and read by the `scope-*` builtins. See `vm::task`.
	pub(crate) sched: crate::task::Scheduler,
	// Opt-in opcode-frequency profiling.
	pub profile: Option<std::collections::HashMap<&'static str, u64>>,
}

impl VM {
	pub fn new(program: Program) -> Self {
		let uses_raw = program.uses_raw;
		Self {
			program,
			stdout: OutputSink::Stdout,
			stderr: OutputSink::Stderr,
			stdin: InputSource::Stdin,
			args: Vec::new(),
			stack: Vec::with_capacity(256),
			// Only the raw (unboxed-I64) path allocates this; empty + untouched when
			// `uses_raw` is false (the default — see `uses_raw`).
			raw: if uses_raw {
				Vec::with_capacity(256)
			} else {
				Vec::new()
			},
			uses_raw,
			frames: Vec::with_capacity(64),
			sched: crate::task::Scheduler::default(),
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
		self.push_frame_with_args(entry, Rc::new(Vec::new()), Vec::new())?;
		self.run_until_frame_depth(0)?;
		let value = self
			.pop_stack()
			.ok_or_else(|| RuntimeError::new("VM exited with empty stack"))?;
		// Lazy runtime init: a purely-synchronous program returns a plain value
		// and never touches the event loop. If `main` returned a task, drive it.
		let value = if matches!(value, Value::Task(_)) {
			self.run_task(value)?
		} else {
			value
		};
		// `main`'s `err e` return doubles as a nonzero exit with `e` on stderr.
		if let Value::Variant(v) = &value {
			if v.variant.as_str() == "err" && v.payload.len() == 1 {
				return Err(RuntimeError::user_abort(format!("{}", v.payload[0])));
			}
		}
		Ok(value)
	}

	// Force a top-level global to its value, evaluating its thunk on first
	// access. Public so external runners can resolve a specific def by index.
	pub fn force_global(&mut self, global_idx: u32) -> Result<Value, RuntimeError> {
		self.load_global(global_idx)
	}

	// Call a callable value (closure / builtin / variant constructor) with
	// `args` and return its result. The external + re-entrant (`builtin::invoke`)
	// entry point: pushes a frame with `ret_dst: None` so its Return pushes the
	// result, which we pop.
	pub fn call_function(&mut self, callee: Value, args: Vec<Value>) -> Result<Value, RuntimeError> {
		match callee {
			Value::Closure(c) => {
				let depth = self.frames.len();
				self.push_frame_with_args(c.fn_idx as u32, Rc::clone(&c.captures), args)?;
				self.run_until_frame_depth(depth)?;
				self
					.pop_stack()
					.ok_or_else(|| RuntimeError::new("VM: call returned with empty stack"))
			}
			Value::Builtin(b) => crate::builtin::call_builtin(self, b.as_ref(), args),
			Value::VariantCtor(c) => Ok(Value::Variant(Rc::new(VariantData {
				qualified_enum: c.qualified_enum.clone(),
				variant: c.variant.clone(),
				payload: args,
			}))),
			Value::AsyncFn(af) => {
				let func = &self.program.functions[af.step_fn];
				let args = if func.param_count == 0 && args.len() == 1 && matches!(args[0], Value::Nothing)
				{
					Vec::new()
				} else {
					args
				};
				Ok(Value::Task(Rc::new(TaskRepr::Async {
					step_fn: af.step_fn,
					captures: Rc::clone(&af.captures),
					args,
				})))
			}
			_ => Err(RuntimeError::new("VM: value is not callable")),
		}
	}

	// Push a frame whose args are passed as a Vec, with `ret_dst: None` (its
	// Return pushes the result). Used by the entry, lazy global thunks, builtin
	// re-entry, and the async drivers. Dispatch-loop calls use `do_call`.
	pub(crate) fn push_frame_with_args(
		&mut self,
		fn_idx: u32,
		captures: Rc<Vec<Value>>,
		args: Vec<Value>,
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
		let base = self.stack.len();
		let nregs = func.nregs as usize;
		self.stack.extend(args);
		self.stack.resize(base + nregs, Value::Nothing);
		if self.uses_raw {
			self.raw.resize(self.stack.len(), 0);
		}
		self.frames.push(Frame {
			fn_idx,
			ip: 0,
			base,
			nregs: nregs as u16,
			captures,
			ret_dst: None,
			cleanups: Vec::new(),
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

	fn current_module(&self) -> Option<String> {
		let frame = self.frames.last()?;
		let func = &self.program.functions[frame.fn_idx as usize];
		if func.module.is_empty() {
			None
		} else {
			Some(func.module.clone())
		}
	}

	// Run until self.frames.len() == target_depth. Two nested loops so the hot
	// path pays no frame re-derivation: the outer loop re-syncs `base`/`fn_idx`/
	// body length after every control transfer (Call/Return/TailCall); the inner
	// loop runs straight-line instructions of one frame with that cache held in
	// locals.
	pub(crate) fn run_until_frame_depth(&mut self, target_depth: usize) -> Result<(), RuntimeError> {
		while self.frames.len() > target_depth {
			let frame_idx = self.frames.len() - 1;
			let base = self.frames[frame_idx].base;
			let fn_idx = self.frames[frame_idx].fn_idx as usize;
			let body_len = self.program.functions[fn_idx].body.len();
			loop {
				let ip = self.frames[frame_idx].ip;
				if ip >= body_len {
					return Err(
						RuntimeError::new("VM: ran past end of function (missing Return?)")
							.at(self.current_range()),
					);
				}
				let instr = self.program.functions[fn_idx].body[ip];
				self.frames[frame_idx].ip = ip + 1;

				if let Some(p) = &mut self.profile {
					*p.entry(opcode_name(&instr)).or_insert(0) += 1;
				}

				if let Flow::Transfer = self.exec_instr(instr, frame_idx, base)? {
					break;
				}
			}
		}
		Ok(())
	}

	// Execute one instruction (used by the async single-stepper `drive_step`).
	pub(crate) fn step(&mut self) -> Result<(), RuntimeError> {
		let frame_idx = self.frames.len() - 1;
		let fn_idx = self.frames[frame_idx].fn_idx as usize;
		let ip = self.frames[frame_idx].ip;
		if ip >= self.program.functions[fn_idx].body.len() {
			return Err(
				RuntimeError::new("VM: ran past end of function (missing Return?)")
					.at(self.current_range()),
			);
		}
		let instr = self.program.functions[fn_idx].body[ip];
		self.frames[frame_idx].ip = ip + 1;

		if let Some(p) = &mut self.profile {
			*p.entry(opcode_name(&instr)).or_insert(0) += 1;
		}

		let base = self.frames[frame_idx].base;
		self.exec_instr(instr, frame_idx, base)?;
		Ok(())
	}

	// Read the operand-register list `idx` into owned values (in list order).
	fn read_reg_list(&self, idx: RegListIdx, base: usize) -> Vec<Value> {
		self.program.reg_lists[idx as usize]
			.iter()
			.map(|r| self.stack[base + *r as usize].clone())
			.collect()
	}

	// `#[inline(always)]` is load-bearing: the dispatch is the innermost loop and
	// the win is that the match body lands directly inside the caller's loop.
	#[inline(always)]
	fn exec_instr(
		&mut self,
		instr: Instruction,
		frame_idx: usize,
		base: usize,
	) -> Result<Flow, RuntimeError> {
		// Integer / float / comparison binops: clone both operands out (owned, so
		// the match arms can call `self.current_range()` and write `dst` freely),
		// then write the result register.
		macro_rules! arith_int {
			($dst:expr, $a:expr, $b:expr, $m:ident, $name:literal) => {{
				let l = self.stack[base + $a as usize].clone();
				let r = self.stack[base + $b as usize].clone();
				let res = match (l, r) {
					(Value::Int(x), Value::Int(y)) => Value::Int(x.$m(y)),
					_ => {
						return Err(
							RuntimeError::new(concat!($name, ": expected ints")).at(self.current_range()),
						);
					}
				};
				self.stack[base + $dst as usize] = res;
			}};
		}
		macro_rules! arith_float {
			($dst:expr, $a:expr, $b:expr, $op:tt, $name:literal) => {{
				let l = self.stack[base + $a as usize].clone();
				let r = self.stack[base + $b as usize].clone();
				let res = match (l, r) {
					(Value::Float(x), Value::Float(y)) => Value::Float(x $op y),
					_ => return Err(RuntimeError::new(concat!($name, ": expected floats")).at(self.current_range())),
				};
				self.stack[base + $dst as usize] = res;
			}};
		}
		macro_rules! cmp_int {
			($dst:expr, $a:expr, $b:expr, $op:tt, $name:literal) => {{
				let l = self.stack[base + $a as usize].clone();
				let r = self.stack[base + $b as usize].clone();
				let res = match (l, r) {
					(Value::Int(x), Value::Int(y)) => Value::Bool(x $op y),
					_ => return Err(RuntimeError::new(concat!($name, ": expected ints")).at(self.current_range())),
				};
				self.stack[base + $dst as usize] = res;
			}};
		}
		macro_rules! cmp_float {
			($dst:expr, $a:expr, $b:expr, $op:tt, $name:literal) => {{
				let l = self.stack[base + $a as usize].clone();
				let r = self.stack[base + $b as usize].clone();
				let res = match (l, r) {
					(Value::Float(x), Value::Float(y)) => Value::Bool(x $op y),
					_ => return Err(RuntimeError::new(concat!($name, ": expected floats")).at(self.current_range())),
				};
				self.stack[base + $dst as usize] = res;
			}};
		}

		let mut flow = Flow::Next;
		match instr {
			Instruction::Move { dst, src } => {
				self.stack[base + dst as usize] = self.stack[base + src as usize].clone();
			}
			// M5 repr boundary: box a raw i64 into a `Value::Int`, or unbox a
			// `Value::Int` into the raw i64 window.
			Instruction::Box { dst, src } => {
				self.stack[base + dst as usize] = Value::Int(self.raw[base + src as usize] as i64);
			}
			Instruction::Unbox { dst, src } => {
				let n = match &self.stack[base + src as usize] {
					Value::Int(n) => *n,
					_ => return Err(RuntimeError::new("Unbox: expected int").at(self.current_range())),
				};
				self.raw[base + dst as usize] = n as u64;
			}
			Instruction::MoveR { dst, src } => {
				self.raw[base + dst as usize] = self.raw[base + src as usize];
			}
			Instruction::LoadConst { dst, k } => {
				let s = self.program.constants[k as usize].clone();
				self.stack[base + dst as usize] = Value::String(s);
			}
			Instruction::LoadBytes { dst, k } => {
				let b = self.program.bytes_constants[k as usize].clone();
				self.stack[base + dst as usize] = Value::Bytes(b);
			}
			Instruction::LoadInt { dst, val } => self.stack[base + dst as usize] = Value::Int(val),
			Instruction::LoadIntR { dst, val } => self.raw[base + dst as usize] = val as u64,
			Instruction::LoadFloat { dst, val } => self.stack[base + dst as usize] = Value::Float(val),
			Instruction::LoadBool { dst, val } => self.stack[base + dst as usize] = Value::Bool(val),
			Instruction::LoadDuration { dst, ns } => {
				self.stack[base + dst as usize] = Value::Duration(ns)
			}
			Instruction::LoadNothing { dst } => self.stack[base + dst as usize] = Value::Nothing,
			Instruction::LoadCapture { dst, idx } => {
				let v = self.frames[frame_idx].captures[idx as usize].clone();
				self.stack[base + dst as usize] = v;
			}
			Instruction::LoadGlobal { dst, idx } => {
				let v = self.load_global(idx)?;
				self.stack[base + dst as usize] = v;
			}
			Instruction::Jump { target } => {
				self.frames[frame_idx].ip = target as usize;
			}
			Instruction::JumpIfFalse { cond, target } => {
				let take = match &self.stack[base + cond as usize] {
					Value::Bool(b) => Some(!*b),
					_ => None,
				};
				match take {
					Some(true) => self.frames[frame_idx].ip = target as usize,
					Some(false) => {}
					None => {
						return Err(
							RuntimeError::new("VM: JumpIfFalse with non-bool").at(self.current_range()),
						);
					}
				}
			}
			Instruction::MakeClosure {
				dst,
				fn_idx,
				captures,
			} => {
				let caps = self.read_reg_list(captures, base);
				self.stack[base + dst as usize] = Value::Closure(Rc::new(ClosureData {
					fn_idx: fn_idx as usize,
					captures: Rc::new(caps),
				}));
			}
			Instruction::MakeAsyncClosure {
				dst,
				fn_idx,
				captures,
			} => {
				let caps = self.read_reg_list(captures, base);
				self.stack[base + dst as usize] = Value::AsyncFn(Rc::new(AsyncFnData {
					step_fn: fn_idx as usize,
					captures: Rc::new(caps),
				}));
			}
			Instruction::Call { dst, callee, args } => {
				let callee_val = self.stack[base + callee as usize].clone();
				self.do_call(dst, callee_val, args, base, false)?;
				flow = Flow::Transfer;
			}
			Instruction::CallDirect { dst, fn_idx, args } => {
				self.enter_closure(fn_idx, Rc::new(Vec::new()), args, dst, base, false)?;
				flow = Flow::Transfer;
			}
			Instruction::TailCall { dst, callee, args } => {
				let callee_val = self.stack[base + callee as usize].clone();
				// For a closure callee the frame is reused (inheriting `ret_dst`)
				// and `dst` is ignored; for a builtin/ctor/async callee the value
				// lands in `dst` for the following `Return(dst)` to deliver.
				self.do_call(dst, callee_val, args, base, true)?;
				flow = Flow::Transfer;
			}
			Instruction::TailCallDirect { dst, fn_idx, args } => {
				self.enter_closure(fn_idx, Rc::new(Vec::new()), args, dst, base, true)?;
				flow = Flow::Transfer;
			}
			Instruction::Return { src, raw: raw_ret } => {
				// A monomorphized function can return an unboxed i64 (M6): read the
				// result from the window its repr names, and deliver it to the
				// caller's `dst` in the same window (`dst`'s repr matches by coercion).
				// `raw_ret` implies `uses_raw`, so the raw read/truncate only run when
				// the raw window is live.
				let result = self.stack[base + src as usize].clone();
				let result_raw = if raw_ret {
					self.raw[base + src as usize]
				} else {
					0
				};
				// Run `defer` cleanups LIFO before tearing down the frame.
				let cleanups = std::mem::take(&mut self.frames[frame_idx].cleanups);
				for thunk in cleanups.into_iter().rev() {
					self.call_function(thunk, Vec::new())?;
				}
				let popped = self.frames.pop().unwrap();
				self.stack.truncate(popped.base);
				if self.uses_raw {
					self.raw.truncate(popped.base);
				}
				match popped.ret_dst {
					Some(abs) => {
						if raw_ret {
							self.raw[abs] = result_raw;
						} else {
							self.stack[abs] = result;
						}
					}
					None => {
						// External callers pop a boxed `Value`; box a raw return.
						self.stack.push(if raw_ret {
							Value::Int(result_raw as i64)
						} else {
							result
						});
						if self.uses_raw {
							self.raw.push(0);
						}
					}
				}
				flow = Flow::Transfer;
			}
			Instruction::PushDefer { thunk } => {
				let t = self.stack[base + thunk as usize].clone();
				self.frames[frame_idx].cleanups.push(t);
			}
			Instruction::Await { .. } => {
				// Await is intercepted by `drive_step` before the normal loop
				// reaches it. Seeing it here is a driver/codegen bug.
				return Err(
					RuntimeError::new("VM: `Await` executed outside the task driver")
						.at(self.current_range()),
				);
			}
			Instruction::MakeTuple { dst, items } => {
				let elems = self.read_reg_list(items, base);
				self.stack[base + dst as usize] = Value::Tuple(Rc::new(elems));
			}
			Instruction::MakeList { dst, items } => {
				let elems = self.read_reg_list(items, base);
				self.stack[base + dst as usize] = Value::list(elems);
			}
			Instruction::ConcatLists { dst, lists } => {
				let regs = self.program.reg_lists[lists as usize].clone();
				let mut out: Vec<Value> = Vec::new();
				for r in regs {
					match &self.stack[base + r as usize] {
						Value::List(xs) => out.extend(xs.borrow().iter().cloned()),
						_ => {
							return Err(
								RuntimeError::new("ConcatLists: expected lists").at(self.current_range()),
							);
						}
					}
				}
				self.stack[base + dst as usize] = Value::list(out);
			}
			Instruction::MakeRecord {
				dst,
				values,
				fields,
			} => {
				let vals = self.read_reg_list(values, base);
				let names = &self.program.field_lists[fields as usize];
				let mut map = std::collections::HashMap::with_capacity(vals.len());
				for (i, v) in vals.into_iter().enumerate() {
					let name = self.program.constants[names[i] as usize].clone();
					map.insert((*name).clone(), v);
				}
				self.stack[base + dst as usize] = Value::Record(Rc::new(map));
			}
			Instruction::UpdateRecord {
				dst,
				record,
				values,
				fields,
			} => {
				let base_rec = self.stack[base + record as usize].clone();
				let vals = self.read_reg_list(values, base);
				let Value::Record(base_map) = base_rec else {
					return Err(
						RuntimeError::new("VM: UpdateRecord on non-record value").at(self.current_range()),
					);
				};
				let names = &self.program.field_lists[fields as usize];
				let mut map = (*base_map).clone();
				for (i, v) in vals.into_iter().enumerate() {
					let name = self.program.constants[names[i] as usize].clone();
					map.insert((*name).clone(), v);
				}
				self.stack[base + dst as usize] = Value::Record(Rc::new(map));
			}
			Instruction::MakeVariant {
				dst,
				qualified,
				variant,
				payload,
			} => {
				let payload = self.read_reg_list(payload, base);
				self.stack[base + dst as usize] = Value::Variant(Rc::new(VariantData {
					qualified_enum: self.program.constants[qualified as usize].clone(),
					variant: self.program.constants[variant as usize].clone(),
					payload,
				}));
			}
			Instruction::MakeVariantCtor {
				dst,
				qualified,
				variant,
				arity,
			} => {
				self.stack[base + dst as usize] = Value::VariantCtor(Rc::new(VariantCtorData {
					qualified_enum: self.program.constants[qualified as usize].clone(),
					variant: self.program.constants[variant as usize].clone(),
					arity: arity as usize,
				}));
			}
			Instruction::GetField { dst, record, name } => {
				let rec = self.stack[base + record as usize].clone();
				let name_s = self.program.constants[name as usize].clone();
				match rec {
					Value::Record(fields) => match fields.get(name_s.as_str()) {
						Some(v) => self.stack[base + dst as usize] = v.clone(),
						None => {
							return Err(
								RuntimeError::new(format!("no field `{}` on record", name_s))
									.at(self.current_range()),
							);
						}
					},
					_ => {
						return Err(
							RuntimeError::new(format!("field access `.{}` on non-record value", name_s))
								.at(self.current_range()),
						);
					}
				}
			}
			Instruction::GetElement { dst, tuple, index } => {
				let t = self.stack[base + tuple as usize].clone();
				match t {
					Value::Tuple(elems) => match elems.get(index as usize) {
						Some(v) => self.stack[base + dst as usize] = v.clone(),
						None => {
							return Err(
								RuntimeError::new(format!(
									"tuple index {} out of bounds (len {})",
									index,
									elems.len()
								))
								.at(self.current_range()),
							);
						}
					},
					_ => {
						return Err(
							RuntimeError::new(format!("element access `.{}` on non-tuple value", index))
								.at(self.current_range()),
						);
					}
				}
			}
			Instruction::GetDictField { dst, dict, index } => {
				let d = self.stack[base + dict as usize].clone();
				match d {
					Value::MethodDict(methods) => {
						let m = methods.get(index as usize).ok_or_else(|| {
							RuntimeError::new(format!(
								"VM: GetDictField index {} out of range (dict size {})",
								index,
								methods.len()
							))
							.at(self.current_range())
						})?;
						self.stack[base + dst as usize] = m.clone();
					}
					_ => {
						return Err(
							RuntimeError::new("VM: GetDictField on non-dict value").at(self.current_range()),
						);
					}
				}
			}
			Instruction::MakeDict { dst, methods } => {
				let ms = self.read_reg_list(methods, base);
				self.stack[base + dst as usize] = Value::MethodDict(Rc::new(ms));
			}
			Instruction::Interpolate { dst, parts } => {
				let ps = self.read_reg_list(parts, base);
				let mut out = String::new();
				for p in &ps {
					match p {
						Value::String(s) => out.push_str(s),
						other => out.push_str(&format!("{}", other)),
					}
				}
				self.stack[base + dst as usize] = Value::String(Rc::new(out));
			}
			Instruction::MatchInt {
				subject,
				val,
				on_fail,
			} => {
				let ok = matches!(&self.stack[base + subject as usize], Value::Int(x) if *x == val);
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchFloat {
				subject,
				val,
				on_fail,
			} => {
				let ok = matches!(&self.stack[base + subject as usize], Value::Float(x) if *x == val);
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchDuration {
				subject,
				ns,
				on_fail,
			} => {
				let ok = matches!(&self.stack[base + subject as usize], Value::Duration(x) if *x == ns);
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchString {
				subject,
				k,
				on_fail,
			} => {
				let needle = self.program.constants[k as usize].clone();
				let ok = matches!(&self.stack[base + subject as usize], Value::String(s) if s.as_ref() == needle.as_ref());
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchBytes {
				subject,
				k,
				on_fail,
			} => {
				let needle = self.program.bytes_constants[k as usize].clone();
				let ok = matches!(&self.stack[base + subject as usize], Value::Bytes(b) if b.as_ref() == needle.as_ref());
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchBool {
				subject,
				val,
				on_fail,
			} => {
				let ok = matches!(&self.stack[base + subject as usize], Value::Bool(x) if *x == val);
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchNothing { subject, on_fail } => {
				let ok = matches!(&self.stack[base + subject as usize], Value::Nothing);
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
				}
			}
			Instruction::MatchVariant {
				subject,
				variant,
				dests,
				on_fail,
			} => self.match_variant(frame_idx, base, subject, variant, dests, on_fail)?,
			Instruction::MatchTuple {
				subject,
				dests,
				on_fail,
			} => self.match_tuple(frame_idx, base, subject, dests, on_fail)?,
			Instruction::MatchList {
				subject,
				dests,
				has_rest,
				on_fail,
			} => self.match_list(frame_idx, base, subject, dests, has_rest, on_fail)?,
			Instruction::MatchRecord {
				subject,
				fields,
				dests,
				exact,
				with_rest,
				on_fail,
			} => self.match_record(
				frame_idx, base, subject, fields, dests, exact, with_rest, on_fail,
			)?,
			Instruction::AddInt { dst, a, b } => arith_int!(dst, a, b, wrapping_add, "AddInt"),
			Instruction::SubInt { dst, a, b } => arith_int!(dst, a, b, wrapping_sub, "SubInt"),
			Instruction::MulInt { dst, a, b } => arith_int!(dst, a, b, wrapping_mul, "MulInt"),
			Instruction::DivInt { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				let res = match (l, r) {
					(Value::Int(_), Value::Int(0)) => {
						return Err(RuntimeError::new("integer division by zero").at(self.current_range()));
					}
					(Value::Int(x), Value::Int(y)) => Value::Int(x.wrapping_div(y)),
					_ => return Err(RuntimeError::new("DivInt: expected ints").at(self.current_range())),
				};
				self.stack[base + dst as usize] = res;
			}
			Instruction::RemInt { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				let res = match (l, r) {
					(Value::Int(_), Value::Int(0)) => {
						return Err(RuntimeError::new("division by zero").at(self.current_range()));
					}
					(Value::Int(x), Value::Int(y)) => Value::Int(x % y),
					_ => return Err(RuntimeError::new("RemInt: expected ints").at(self.current_range())),
				};
				self.stack[base + dst as usize] = res;
			}
			Instruction::AddFloat { dst, a, b } => arith_float!(dst, a, b, +, "AddFloat"),
			Instruction::SubFloat { dst, a, b } => arith_float!(dst, a, b, -, "SubFloat"),
			Instruction::MulFloat { dst, a, b } => arith_float!(dst, a, b, *, "MulFloat"),
			Instruction::DivFloat { dst, a, b } => arith_float!(dst, a, b, /, "DivFloat"),
			Instruction::RemFloat { dst, a, b } => arith_float!(dst, a, b, %, "RemFloat"),
			Instruction::NegInt { dst, a } => {
				let v = self.stack[base + a as usize].clone();
				match v {
					Value::Int(n) => self.stack[base + dst as usize] = Value::Int(n.wrapping_neg()),
					_ => return Err(RuntimeError::new("NegInt: expected int").at(self.current_range())),
				}
			}
			Instruction::NegFloat { dst, a } => {
				let v = self.stack[base + a as usize].clone();
				match v {
					Value::Float(n) => self.stack[base + dst as usize] = Value::Float(-n),
					_ => return Err(RuntimeError::new("NegFloat: expected float").at(self.current_range())),
				}
			}
			Instruction::ConcatString { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				match (l, r) {
					(Value::String(x), Value::String(y)) => {
						let mut s = String::with_capacity(x.len() + y.len());
						s.push_str(&x);
						s.push_str(&y);
						self.stack[base + dst as usize] = Value::String(Rc::new(s));
					}
					_ => {
						return Err(
							RuntimeError::new("ConcatString: expected strings").at(self.current_range()),
						);
					}
				}
			}
			Instruction::LtInt { dst, a, b } => cmp_int!(dst, a, b, <, "LtInt"),
			Instruction::LtFloat { dst, a, b } => cmp_float!(dst, a, b, <, "LtFloat"),
			Instruction::LteInt { dst, a, b } => cmp_int!(dst, a, b, <=, "LteInt"),
			Instruction::LteFloat { dst, a, b } => cmp_float!(dst, a, b, <=, "LteFloat"),
			Instruction::GtInt { dst, a, b } => cmp_int!(dst, a, b, >, "GtInt"),
			Instruction::GtFloat { dst, a, b } => cmp_float!(dst, a, b, >, "GtFloat"),
			Instruction::GteInt { dst, a, b } => cmp_int!(dst, a, b, >=, "GteInt"),
			Instruction::GteFloat { dst, a, b } => cmp_float!(dst, a, b, >=, "GteFloat"),
			Instruction::Eq { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				self.stack[base + dst as usize] = Value::Bool(values_eq(&l, &r));
			}
			Instruction::Neq { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				self.stack[base + dst as usize] = Value::Bool(!values_eq(&l, &r));
			}
			Instruction::LogicalAnd { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				match (l, r) {
					(Value::Bool(x), Value::Bool(y)) => self.stack[base + dst as usize] = Value::Bool(x && y),
					_ => return Err(RuntimeError::new("expected bools for `&&`").at(self.current_range())),
				}
			}
			Instruction::LogicalOr { dst, a, b } => {
				let l = self.stack[base + a as usize].clone();
				let r = self.stack[base + b as usize].clone();
				match (l, r) {
					(Value::Bool(x), Value::Bool(y)) => self.stack[base + dst as usize] = Value::Bool(x || y),
					_ => return Err(RuntimeError::new("expected bools for `||`").at(self.current_range())),
				}
			}
			Instruction::LogicalNot { dst, a } => {
				let v = self.stack[base + a as usize].clone();
				match v {
					Value::Bool(x) => self.stack[base + dst as usize] = Value::Bool(!x),
					_ => return Err(RuntimeError::new("expected bool for `!`").at(self.current_range())),
				}
			}

			// --- M5: unboxed i64 arithmetic/comparison ------------------------
			// Operands (and arithmetic dsts) are raw i64; no `Value`, no clone, no
			// tag check. The repr pass proved the operands are i64. Comparisons
			// write a boxed `Value::Bool`.
			Instruction::AddIntR { dst, a, b } => {
				self.raw[base + dst as usize] = (self.raw[base + a as usize] as i64)
					.wrapping_add(self.raw[base + b as usize] as i64)
					as u64;
			}
			Instruction::SubIntR { dst, a, b } => {
				self.raw[base + dst as usize] = (self.raw[base + a as usize] as i64)
					.wrapping_sub(self.raw[base + b as usize] as i64)
					as u64;
			}
			Instruction::MulIntR { dst, a, b } => {
				self.raw[base + dst as usize] = (self.raw[base + a as usize] as i64)
					.wrapping_mul(self.raw[base + b as usize] as i64)
					as u64;
			}
			Instruction::DivIntR { dst, a, b } => {
				let d = self.raw[base + b as usize] as i64;
				if d == 0 {
					return Err(RuntimeError::new("integer division by zero").at(self.current_range()));
				}
				self.raw[base + dst as usize] = (self.raw[base + a as usize] as i64).wrapping_div(d) as u64;
			}
			Instruction::RemIntR { dst, a, b } => {
				let d = self.raw[base + b as usize] as i64;
				if d == 0 {
					return Err(RuntimeError::new("division by zero").at(self.current_range()));
				}
				self.raw[base + dst as usize] = ((self.raw[base + a as usize] as i64) % d) as u64;
			}
			Instruction::NegIntR { dst, a } => {
				self.raw[base + dst as usize] = (self.raw[base + a as usize] as i64).wrapping_neg() as u64;
			}
			Instruction::LtIntR { dst, a, b } => {
				self.stack[base + dst as usize] =
					Value::Bool((self.raw[base + a as usize] as i64) < (self.raw[base + b as usize] as i64));
			}
			Instruction::LteIntR { dst, a, b } => {
				self.stack[base + dst as usize] =
					Value::Bool((self.raw[base + a as usize] as i64) <= (self.raw[base + b as usize] as i64));
			}
			Instruction::GtIntR { dst, a, b } => {
				self.stack[base + dst as usize] =
					Value::Bool((self.raw[base + a as usize] as i64) > (self.raw[base + b as usize] as i64));
			}
			Instruction::GteIntR { dst, a, b } => {
				self.stack[base + dst as usize] =
					Value::Bool((self.raw[base + a as usize] as i64) >= (self.raw[base + b as usize] as i64));
			}
		}
		Ok(flow)
	}

	fn match_variant(
		&mut self,
		frame_idx: usize,
		base: usize,
		subject: Reg,
		variant_idx: u32,
		dests: RegListIdx,
		on_fail: u32,
	) -> Result<(), RuntimeError> {
		let subj = self.stack[base + subject as usize].clone();
		let variant_name = self.program.constants[variant_idx as usize].clone();
		let dest_regs = self.program.reg_lists[dests as usize].clone();
		match subj {
			Value::Variant(v)
				if v.variant.as_ref() == variant_name.as_ref() && v.payload.len() == dest_regs.len() =>
			{
				for (i, d) in dest_regs.iter().enumerate() {
					self.stack[base + *d as usize] = v.payload[i].clone();
				}
			}
			Value::Bool(true) if variant_name.as_ref() == "true" && dest_regs.is_empty() => {}
			Value::Bool(false) if variant_name.as_ref() == "false" && dest_regs.is_empty() => {}
			_ => self.frames[frame_idx].ip = on_fail as usize,
		}
		Ok(())
	}

	fn match_tuple(
		&mut self,
		frame_idx: usize,
		base: usize,
		subject: Reg,
		dests: RegListIdx,
		on_fail: u32,
	) -> Result<(), RuntimeError> {
		let subj = self.stack[base + subject as usize].clone();
		let dest_regs = self.program.reg_lists[dests as usize].clone();
		match subj {
			Value::Tuple(elems) if elems.len() == dest_regs.len() => {
				for (i, d) in dest_regs.iter().enumerate() {
					self.stack[base + *d as usize] = elems[i].clone();
				}
			}
			_ => self.frames[frame_idx].ip = on_fail as usize,
		}
		Ok(())
	}

	fn match_list(
		&mut self,
		frame_idx: usize,
		base: usize,
		subject: Reg,
		dests: RegListIdx,
		has_rest: bool,
		on_fail: u32,
	) -> Result<(), RuntimeError> {
		let subj = self.stack[base + subject as usize].clone();
		let dest_regs = self.program.reg_lists[dests as usize].clone();
		let items = dest_regs.len() - has_rest as usize;
		match subj {
			Value::List(elems) => {
				let elems = elems.borrow();
				let len = elems.len();
				let ok = if has_rest { len >= items } else { len == items };
				if !ok {
					self.frames[frame_idx].ip = on_fail as usize;
					return Ok(());
				}
				for i in 0..items {
					self.stack[base + dest_regs[i] as usize] = elems[i].clone();
				}
				if has_rest {
					let tail: Vec<Value> = elems[items..].to_vec();
					self.stack[base + dest_regs[items] as usize] = Value::list(tail);
				}
			}
			_ => self.frames[frame_idx].ip = on_fail as usize,
		}
		Ok(())
	}

	#[allow(clippy::too_many_arguments)]
	fn match_record(
		&mut self,
		frame_idx: usize,
		base: usize,
		subject: Reg,
		fields_idx: u32,
		dests: RegListIdx,
		exact: bool,
		with_rest: bool,
		on_fail: u32,
	) -> Result<(), RuntimeError> {
		let subj = self.stack[base + subject as usize].clone();
		let names = self.program.field_lists[fields_idx as usize].clone();
		let dest_regs = self.program.reg_lists[dests as usize].clone();
		let n = names.len();
		let Value::Record(record) = subj else {
			self.frames[frame_idx].ip = on_fail as usize;
			return Ok(());
		};
		if exact && record.len() != n {
			self.frames[frame_idx].ip = on_fail as usize;
			return Ok(());
		}
		let mut values = Vec::with_capacity(n);
		let mut matched: std::collections::HashSet<String> =
			std::collections::HashSet::with_capacity(n);
		for &name_idx in &names {
			let name = self.program.constants[name_idx as usize].clone();
			match record.get(name.as_str()) {
				Some(v) => {
					values.push(v.clone());
					matched.insert((*name).clone());
				}
				None => {
					self.frames[frame_idx].ip = on_fail as usize;
					return Ok(());
				}
			}
		}
		for (i, v) in values.into_iter().enumerate() {
			self.stack[base + dest_regs[i] as usize] = v;
		}
		if with_rest {
			let mut rest: std::collections::HashMap<String, Value> =
				std::collections::HashMap::with_capacity(record.len().saturating_sub(n));
			for (k, v) in record.iter() {
				if !matched.contains(k) {
					rest.insert(k.clone(), v.clone());
				}
			}
			self.stack[base + dest_regs[n] as usize] = Value::Record(Rc::new(rest));
		}
		Ok(())
	}

	fn load_global(&mut self, idx: u32) -> Result<Value, RuntimeError> {
		match &self.program.globals[idx as usize] {
			GlobalSlot::Evaluated(v) => Ok(v.clone()),
			GlobalSlot::Evaluating => Err(
				RuntimeError::new(format!("cycle detected while evaluating global #{}", idx))
					.at(self.current_range()),
			),
			GlobalSlot::Pending(fn_idx) => {
				let fn_idx = *fn_idx;
				self.program.globals[idx as usize] = GlobalSlot::Evaluating;
				let depth = self.frames.len();
				self.push_frame_with_args(fn_idx, Rc::new(Vec::new()), Vec::new())?;
				self.run_until_frame_depth(depth)?;
				let v = self
					.pop_stack()
					.ok_or_else(|| RuntimeError::new("VM: global thunk produced no value"))?;
				self.program.globals[idx as usize] = GlobalSlot::Evaluated(v.clone());
				Ok(v)
			}
		}
	}

	// Whether parameter `i` of function `fn_idx` is an unboxed i64 (so a call
	// must marshal that arg through the raw window). M6.
	fn param_is_raw(&self, fn_idx: u32, i: usize) -> bool {
		matches!(
			self.program.functions[fn_idx as usize].reg_reprs.get(i),
			Some(RegRepr::I64)
		)
	}

	// Dispatch a call to `callee`, delivering the result to register `dst` of the
	// caller (whose window starts at `caller_base`). `args` is the operand-list
	// index; closures marshal their args straight from the caller's registers
	// into the new frame (no intermediate `Vec`), while builtins / variant ctors /
	// async fns read the args into a `Vec` and produce their value inline.
	fn do_call(
		&mut self,
		dst: Reg,
		callee: Value,
		args: RegListIdx,
		caller_base: usize,
		tail: bool,
	) -> Result<(), RuntimeError> {
		// A frame with pending `defer` cleanups can't be reused by a tail call:
		// its Return must still run them. Fall back to a normal call.
		let tail = tail && self.frames.last().map_or(true, |f| f.cleanups.is_empty());
		match callee {
			Value::Closure(c) => self.enter_closure(
				c.fn_idx as u32,
				Rc::clone(&c.captures),
				args,
				dst,
				caller_base,
				tail,
			),
			Value::Builtin(b) => {
				let arg_vals = self.read_reg_list(args, caller_base);
				let range = self.current_range();
				let module = self.current_module();
				let result = builtin::call_builtin(self, b.as_ref(), arg_vals).map_err(|e| {
					let mut e = e.at(range);
					if let Some(m) = module {
						e = e.in_module(m);
					}
					e
				})?;
				self.stack[caller_base + dst as usize] = result;
				Ok(())
			}
			Value::VariantCtor(c) => {
				let payload = self.read_reg_list(args, caller_base);
				if payload.len() != c.arity {
					return Err(
						RuntimeError::new(format!(
							"variant `{}.{}` takes {} arg(s), got {}",
							c.qualified_enum
								.rsplit_once('.')
								.map(|(_, n)| n)
								.unwrap_or(&c.qualified_enum),
							c.variant,
							c.arity,
							payload.len()
						))
						.at(self.current_range()),
					);
				}
				self.stack[caller_base + dst as usize] = Value::Variant(Rc::new(VariantData {
					qualified_enum: c.qualified_enum.clone(),
					variant: c.variant.clone(),
					payload,
				}));
				Ok(())
			}
			Value::AsyncFn(af) => {
				let mut args = self.read_reg_list(args, caller_base);
				let func = &self.program.functions[af.step_fn];
				if func.param_count == 0 && args.len() == 1 && matches!(args[0], Value::Nothing) {
					args.clear();
				}
				if args.len() != func.param_count as usize {
					return Err(
						RuntimeError::new(format!(
							"arity mismatch: expected {} args, got {}",
							func.param_count,
							args.len()
						))
						.at(self.current_range()),
					);
				}
				self.stack[caller_base + dst as usize] = Value::Task(Rc::new(TaskRepr::Async {
					step_fn: af.step_fn,
					captures: Rc::clone(&af.captures),
					args,
				}));
				Ok(())
			}
			_ => Err(RuntimeError::new("not callable").at(self.current_range())),
		}
	}

	// Enter a closure (or, when `tail`, replace the current frame in place),
	// marshalling args directly from the caller's registers (named by the operand
	// list `args`) into the callee's parameter registers `0..argc` — no
	// intermediate `Vec` on the hot non-tail path.
	fn enter_closure(
		&mut self,
		fn_idx: u32,
		captures: Rc<Vec<Value>>,
		args: RegListIdx,
		dst: Reg,
		caller_base: usize,
		tail: bool,
	) -> Result<(), RuntimeError> {
		let (param_count, nregs) = {
			let func = &self.program.functions[fn_idx as usize];
			(func.param_count as usize, func.nregs as usize)
		};
		let n = self.program.reg_lists[args as usize].len();
		// Normalize the zero-arg-with-`nothing` call to zero args.
		let argc = if param_count == 0 && n == 1 {
			let r0 = self.program.reg_lists[args as usize][0];
			if matches!(self.stack[caller_base + r0 as usize], Value::Nothing) {
				0
			} else {
				n
			}
		} else {
			n
		};
		if argc != param_count {
			return Err(
				RuntimeError::new(format!(
					"arity mismatch: expected {param_count} args, got {argc}"
				))
				.at(self.current_range()),
			);
		}
		if tail {
			// The args live in the frame we're about to overwrite, so copy them
			// out first (they may alias the destination param slots). Each arg is
			// read in its param's repr (raw i64 or boxed).
			let new_base = self.frames.last().unwrap().base;
			let mut tmp = Vec::with_capacity(argc);
			for i in 0..argc {
				let r = self.program.reg_lists[args as usize][i] as usize;
				if self.param_is_raw(fn_idx, i) {
					tmp.push(Arg::Raw(self.raw[caller_base + r]));
				} else {
					tmp.push(Arg::Boxed(self.stack[caller_base + r].clone()));
				}
			}
			self.stack.truncate(new_base);
			self.stack.resize(new_base + nregs, Value::Nothing);
			if self.uses_raw {
				self.raw.truncate(new_base);
				self.raw.resize(new_base + nregs, 0);
			}
			for (i, a) in tmp.into_iter().enumerate() {
				match a {
					Arg::Raw(b) => self.raw[new_base + i] = b,
					Arg::Boxed(v) => self.stack[new_base + i] = v,
				}
			}
			let frame = self.frames.last_mut().unwrap();
			frame.fn_idx = fn_idx;
			frame.ip = 0;
			frame.captures = captures;
			frame.base = new_base;
			frame.nregs = nregs as u16;
		} else {
			// The callee window sits above the caller's, so reading caller
			// registers while filling it can't overlap — marshal in place, each
			// arg in its param's repr (raw i64 or boxed).
			let new_base = self.stack.len();
			self.stack.resize(new_base + nregs, Value::Nothing);
			if self.uses_raw {
				self.raw.resize(new_base + nregs, 0);
			}
			for i in 0..argc {
				let r = self.program.reg_lists[args as usize][i] as usize;
				if self.param_is_raw(fn_idx, i) {
					self.raw[new_base + i] = self.raw[caller_base + r];
				} else {
					self.stack[new_base + i] = self.stack[caller_base + r].clone();
				}
			}
			self.frames.push(Frame {
				fn_idx,
				ip: 0,
				base: new_base,
				nregs: nregs as u16,
				captures,
				ret_dst: Some(caller_base + dst as usize),
				cleanups: Vec::new(),
			});
		}
		Ok(())
	}
}

// Tiny helpers used by builtin::invoke and the task driver (so VM internals stay
// private to the crate).
impl VM {
	pub(crate) fn frames_len(&self) -> usize {
		self.frames.len()
	}
	pub(crate) fn pop_stack(&mut self) -> Option<Value> {
		if self.uses_raw {
			self.raw.pop();
		}
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

// One marshalled call argument, carried in its destination param's repr: a
// boxed `Value` or a raw i64 (M6 — monomorphized functions take unboxed params).
enum Arg {
	Boxed(Value),
	Raw(u64),
}

// Whether an executed instruction kept control in the current frame (`Next`) or
// transferred it (`Transfer` — Call pushed a frame, Return popped one, TailCall
// replaced the current one).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
	Next,
	Transfer,
}

fn opcode_name(i: &Instruction) -> &'static str {
	use Instruction::*;
	match i {
		Move { .. } => "Move",
		MoveR { .. } => "MoveR",
		Box { .. } => "Box",
		Unbox { .. } => "Unbox",
		LoadConst { .. } => "LoadConst",
		LoadBytes { .. } => "LoadBytes",
		LoadInt { .. } => "LoadInt",
		LoadIntR { .. } => "LoadIntR",
		LoadFloat { .. } => "LoadFloat",
		LoadBool { .. } => "LoadBool",
		LoadDuration { .. } => "LoadDuration",
		LoadNothing { .. } => "LoadNothing",
		LoadCapture { .. } => "LoadCapture",
		LoadGlobal { .. } => "LoadGlobal",
		Jump { .. } => "Jump",
		JumpIfFalse { .. } => "JumpIfFalse",
		MakeClosure { .. } => "MakeClosure",
		MakeAsyncClosure { .. } => "MakeAsyncClosure",
		Call { .. } => "Call",
		CallDirect { .. } => "CallDirect",
		TailCall { .. } => "TailCall",
		TailCallDirect { .. } => "TailCallDirect",
		Return { .. } => "Return",
		PushDefer { .. } => "PushDefer",
		Await { .. } => "Await",
		MakeTuple { .. } => "MakeTuple",
		MakeList { .. } => "MakeList",
		ConcatLists { .. } => "ConcatLists",
		MakeRecord { .. } => "MakeRecord",
		UpdateRecord { .. } => "UpdateRecord",
		MakeVariant { .. } => "MakeVariant",
		MakeVariantCtor { .. } => "MakeVariantCtor",
		GetField { .. } => "GetField",
		GetElement { .. } => "GetElement",
		GetDictField { .. } => "GetDictField",
		MakeDict { .. } => "MakeDict",
		Interpolate { .. } => "Interpolate",
		MatchInt { .. } => "MatchInt",
		MatchFloat { .. } => "MatchFloat",
		MatchDuration { .. } => "MatchDuration",
		MatchString { .. } => "MatchString",
		MatchBytes { .. } => "MatchBytes",
		MatchBool { .. } => "MatchBool",
		MatchNothing { .. } => "MatchNothing",
		MatchVariant { .. } => "MatchVariant",
		MatchTuple { .. } => "MatchTuple",
		MatchList { .. } => "MatchList",
		MatchRecord { .. } => "MatchRecord",
		AddInt { .. } => "AddInt",
		AddFloat { .. } => "AddFloat",
		SubInt { .. } => "SubInt",
		SubFloat { .. } => "SubFloat",
		MulInt { .. } => "MulInt",
		MulFloat { .. } => "MulFloat",
		DivInt { .. } => "DivInt",
		DivFloat { .. } => "DivFloat",
		RemInt { .. } => "RemInt",
		RemFloat { .. } => "RemFloat",
		NegInt { .. } => "NegInt",
		NegFloat { .. } => "NegFloat",
		ConcatString { .. } => "ConcatString",
		LtInt { .. } => "LtInt",
		LtFloat { .. } => "LtFloat",
		LteInt { .. } => "LteInt",
		LteFloat { .. } => "LteFloat",
		GtInt { .. } => "GtInt",
		GtFloat { .. } => "GtFloat",
		GteInt { .. } => "GteInt",
		GteFloat { .. } => "GteFloat",
		Eq { .. } => "Eq",
		Neq { .. } => "Neq",
		LogicalAnd { .. } => "LogicalAnd",
		LogicalOr { .. } => "LogicalOr",
		LogicalNot { .. } => "LogicalNot",
		AddIntR { .. } => "AddIntR",
		SubIntR { .. } => "SubIntR",
		MulIntR { .. } => "MulIntR",
		DivIntR { .. } => "DivIntR",
		RemIntR { .. } => "RemIntR",
		NegIntR { .. } => "NegIntR",
		LtIntR { .. } => "LtIntR",
		LteIntR { .. } => "LteIntR",
		GtIntR { .. } => "GtIntR",
		GteIntR { .. } => "GteIntR",
	}
}
