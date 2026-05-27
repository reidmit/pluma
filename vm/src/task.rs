// The task driver: Pluma's async runtime.
//
// An async-bearing function (one whose body awaits a task via `try`) is
// compiled to a *step function* — its body lowered with an `Await`
// instruction at each suspension point and a heap-saved frame so it can be
// resumed. Calling such a function builds a cold `Value::Task` recipe (see
// `do_call`'s `AsyncFn` arm); this module *runs* one.
//
// The model is the CPS state machine from ASYNC.md, realized in the bytecode
// VM: each running async function is a `TaskFrame` whose locals + live
// operand-stack temporaries are snapshotted on the heap at each `Await`, then
// restored and resumed (jumping straight to the saved ip — the "state tag" is
// just the instruction pointer). Suspension never grows the Rust stack: the
// await chain lives in `act_stack`, not in nested `call_function` calls.
//
// M2 scope: a single fiber driven to completion, timers blocking the thread.
// Concurrency, a microtask queue, and cancellation arrive with `scope` (M4);
// awaited/cancellation cleanup with `defer` arrives in M6.

use crate::value::{TaskRepr, Value};
use crate::vm::{Frame, VM};
use crate::RuntimeError;
use std::rc::Rc;

// What stepping one task frame produced before it yielded control back here.
enum StepOutcome {
	// The step function hit an `Await`: this `Value::Task` must run, and its
	// result (or failure) resumes the frame.
	Await(Value),
	// The step function returned. The value is the function's *tail task* —
	// e.g. `task.return v` produces `Pure(v)`. The driver runs it in tail
	// position (the completed frame is gone).
	Complete(Value),
}

// A running instance of an async function: a resumable bytecode frame whose
// state lives on the heap so it survives suspension. Distinct from the cold
// `TaskRepr::Async` recipe — this is the started, one-shot instance (the
// "task-handle" of ASYNC.md).
struct TaskFrame {
	step_fn: usize,
	captures: Rc<Vec<Value>>,
	// The frame's operand-stack region (slot locals plus any live temporaries
	// from a partially-evaluated expression) saved at the last suspension. On
	// first entry it's the initial locals: the call args padded to slot_count.
	// Restored above `base` on resume — saving the *whole* region (not just
	// the slots) is what lets an `Await` appear mid-expression, e.g.
	// `f (if c { try x = t  ... }) y`.
	saved: Vec<Value>,
	resume_ip: usize,
	// `defer` cleanups accrued in this frame so far. Held here across
	// suspension; moved into the live VM frame while stepping so PushDefer /
	// Return see them. Run LIFO on normal completion (by the VM's Return) and
	// on failure (by `unwind_failure`).
	cleanups: Vec<Value>,
}

impl TaskFrame {
	fn new(step_fn: usize, captures: Rc<Vec<Value>>, mut args: Vec<Value>, vm: &VM) -> Self {
		let slot_count = vm.program.functions[step_fn].slot_count as usize;
		args.resize(slot_count, Value::Nothing);
		TaskFrame {
			step_fn,
			captures,
			saved: args,
			resume_ip: 0,
			cleanups: Vec::new(),
		}
	}
}

// What the driver should do on the next loop turn.
enum Next {
	// Begin running this `Value::Task` (a leaf or a cold async instance).
	Start(Value),
	// Resume the top frame of `act_stack` with this awaited result.
	Resume(Value),
	// The root task finished with this value.
	Done(Value),
}

impl VM {
	// Drive a cold task to completion on this thread. Returns the produced
	// value, or `Err` (a user abort) if the task — or any sub-task it awaited
	// — failed and nothing recovered it. Called by `run()` when `main` returns
	// a task.
	pub(crate) fn run_task(&mut self, root: Value) -> Result<Value, RuntimeError> {
		// The await chain: the top frame is making progress; each frame below
		// is suspended awaiting the one above it.
		let mut act_stack: Vec<TaskFrame> = Vec::new();
		let mut next = Next::Start(root);
		loop {
			match next {
				Next::Done(v) => return Ok(v),
				Next::Start(task) => {
					let repr = match &task {
						Value::Task(r) => Rc::clone(r),
						// A non-task value reaching here would be a type-system
						// violation; treat it as already produced.
						other => {
							next = settle_ok(&mut act_stack, other.clone());
							continue;
						}
					};
					match repr.as_ref() {
						TaskRepr::Pure(v) => next = settle_ok(&mut act_stack, v.clone()),
						TaskRepr::Yield => next = settle_ok(&mut act_stack, Value::Nothing),
						TaskRepr::Sleep(ns) => {
							if *ns > 0 {
								std::thread::sleep(std::time::Duration::from_nanos(*ns as u64));
							}
							next = settle_ok(&mut act_stack, Value::Nothing);
						}
						TaskRepr::Fail(e) => return self.unwind_failure(&mut act_stack, e.clone()),
						TaskRepr::Async {
							step_fn,
							captures,
							args,
						} => {
							let mut tf = TaskFrame::new(*step_fn, Rc::clone(captures), args.clone(), self);
							let outcome = self.drive_step(&mut tf, None)?;
							act_stack.push(tf);
							next = self.after_outcome(&mut act_stack, outcome);
						}
					}
				}
				Next::Resume(v) => {
					let mut tf = act_stack
						.pop()
						.ok_or_else(|| RuntimeError::new("VM: task resume with empty stack"))?;
					let outcome = self.drive_step(&mut tf, Some(v))?;
					act_stack.push(tf);
					next = self.after_outcome(&mut act_stack, outcome);
				}
			}
		}
	}

	// Translate a step's outcome into the next driver action.
	fn after_outcome(&mut self, act_stack: &mut Vec<TaskFrame>, outcome: StepOutcome) -> Next {
		match outcome {
			// Suspended: the top frame stays on `act_stack`; run the awaited
			// sub-task next, then its result resumes the frame.
			StepOutcome::Await(sub) => Next::Start(sub),
			// Done: this frame's cleanups already ran (in the VM's Return).
			// Drop it and run its tail task in its place — settling to the
			// awaiter below, or to `Done` if it was the root.
			StepOutcome::Complete(tail) => {
				act_stack.pop();
				Next::Start(tail)
			}
		}
	}

	// Propagate a task failure up the await chain, running each suspended
	// frame's `defer` cleanups LIFO on the way out (matching how `try`-failure
	// runs defers in the synchronous case). With no recovery combinators yet
	// (M2), this always reaches the top and aborts.
	fn unwind_failure(
		&mut self,
		act_stack: &mut Vec<TaskFrame>,
		err: Value,
	) -> Result<Value, RuntimeError> {
		while let Some(mut tf) = act_stack.pop() {
			let cleanups = std::mem::take(&mut tf.cleanups);
			for thunk in cleanups.into_iter().rev() {
				// A cleanup that itself raises a hard error propagates; a
				// cleanup that fails softly is the caller's concern. Best-effort
				// semantics (M6) refine this once cancellation lands.
				self.call_function(thunk, Vec::new())?;
			}
		}
		Err(RuntimeError::user_abort(format!("{}", err)))
	}

	// Push a VM frame backed by `tf`'s saved state, run it until it `Await`s or
	// returns, then snapshot (on await) or tear down (on return).
	fn drive_step(
		&mut self,
		tf: &mut TaskFrame,
		resume_val: Option<Value>,
	) -> Result<StepOutcome, RuntimeError> {
		let step_fn = tf.step_fn;
		let slot_count = self.program.functions[step_fn].slot_count as usize;
		let target_depth = self.frames.len();
		let base = self.stack.len();
		// Restore the frame's saved region (locals + any live temporaries).
		self.stack.extend(tf.saved.iter().cloned());
		self.frames.push(Frame {
			fn_idx: step_fn as u32,
			ip: tf.resume_ip,
			base,
			slot_count: slot_count as u16,
			prev_top: base,
			captures: Rc::clone(&tf.captures),
			forcing_global: None,
			cleanups: std::mem::take(&mut tf.cleanups),
		});
		// On resume, the awaited result sits on top — where the instruction
		// after `Await` expects it (a StoreLocal binding the `try` pattern, or
		// a Pop for `try _ = ...`).
		if let Some(v) = resume_val {
			self.stack.push(v);
		}

		loop {
			// The step frame returned: its Return already ran cleanups,
			// truncated the stack, and pushed the tail task.
			if self.frames.len() == target_depth {
				let tail = self
					.stack
					.pop()
					.ok_or_else(|| RuntimeError::new("VM: task step returned with empty stack"))?;
				return Ok(StepOutcome::Complete(tail));
			}
			let top = self.frames.len() - 1;
			let frame = &self.frames[top];
			let func = &self.program.functions[frame.fn_idx as usize];
			// Intercept `Await` (only ever present in a step frame, at this
			// depth — nested sync calls contain none). Everything else runs
			// through the normal step loop, including nested calls/returns.
			if frame.ip < func.body.len()
				&& matches!(func.body[frame.ip], crate::instruction::Instruction::Await)
			{
				self.frames[top].ip += 1;
				let awaited = self
					.stack
					.pop()
					.ok_or_else(|| RuntimeError::new("VM: `Await` on empty stack"))?;
				let frame_base = self.frames[top].base;
				tf.saved = self.stack[frame_base..].to_vec();
				tf.resume_ip = self.frames[top].ip;
				let popped = self.frames.pop().unwrap();
				tf.cleanups = popped.cleanups;
				self.stack.truncate(popped.prev_top);
				return Ok(StepOutcome::Await(awaited));
			}
			self.step()?;
		}
	}
}

// Settle a successful value into the await chain: resume the awaiter, or
// finish if this was the root.
fn settle_ok(act_stack: &mut [TaskFrame], v: Value) -> Next {
	if act_stack.is_empty() {
		Next::Done(v)
	} else {
		Next::Resume(v)
	}
}
