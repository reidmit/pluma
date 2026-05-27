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

use crate::value::{TaskRepr, Value, VariantData};
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

// An entry in the await chain. The top is making progress; each entry below
// is suspended waiting on the one above. `Async` frames are *resumed* with a
// settled value; the combinator frames *transform* it (and are popped).
enum Activation {
	// A running async-function instance, suspended at an `Await`.
	Async(TaskFrame),
	// `task.then` continuation: `k : fun a -> task b`, run on success.
	Then(Value),
	// `task.or-else` recovery: `recover : fun nothing -> task a`, run on failure.
	OrElse(Value),
	// `task.attempt`: reify the inner outcome into `ok`/`err`.
	Attempt,
	// `task.map`: apply the pure `f : fun a -> b` to a successful value.
	Map(Value),
}

// What the driver should do on the next loop turn.
enum Next {
	// Begin running this `Value::Task` (a leaf, a combinator, or a cold async
	// instance).
	Start(Value),
	// The root task finished with this value.
	Done(Value),
}

impl VM {
	// Drive a cold task to completion on this thread. Returns the produced
	// value, or `Err` (a user abort) if the task — or any sub-task it awaited
	// — failed and nothing recovered it. Called by `run()` when `main` returns
	// a task.
	pub(crate) fn run_task(&mut self, root: Value) -> Result<Value, RuntimeError> {
		let mut act: Vec<Activation> = Vec::new();
		let mut next = Next::Start(root);
		loop {
			match next {
				Next::Done(v) => return Ok(v),
				Next::Start(task) => next = self.start_task(&mut act, task)?,
			}
		}
	}

	// Begin running one task value: resolve a leaf inline, push a combinator
	// frame and descend, or instantiate + first-step an async function.
	fn start_task(&mut self, act: &mut Vec<Activation>, task: Value) -> Result<Next, RuntimeError> {
		let repr = match &task {
			Value::Task(r) => Rc::clone(r),
			// A non-task value reaching here would be a type-system violation;
			// treat it as already produced.
			other => return self.settle_ok(act, other.clone()),
		};
		match repr.as_ref() {
			TaskRepr::Pure(v) => self.settle_ok(act, v.clone()),
			TaskRepr::Yield => self.settle_ok(act, Value::Nothing),
			TaskRepr::Sleep(ns) => {
				if *ns > 0 {
					std::thread::sleep(std::time::Duration::from_nanos(*ns as u64));
				}
				self.settle_ok(act, Value::Nothing)
			}
			TaskRepr::Fail(e) => self.settle_err(act, e.clone()),
			// Combinators: push a frame that intercepts the inner task's
			// outcome, then run the inner task.
			TaskRepr::Then { task, k } => {
				act.push(Activation::Then(k.clone()));
				Ok(Next::Start((**task).clone()))
			}
			TaskRepr::OrElse { task, recover } => {
				act.push(Activation::OrElse(recover.clone()));
				Ok(Next::Start((**task).clone()))
			}
			TaskRepr::Attempt { task } => {
				act.push(Activation::Attempt);
				Ok(Next::Start((**task).clone()))
			}
			TaskRepr::Map { task, f } => {
				act.push(Activation::Map(f.clone()));
				Ok(Next::Start((**task).clone()))
			}
			TaskRepr::Async {
				step_fn,
				captures,
				args,
			} => {
				let mut tf = TaskFrame::new(*step_fn, Rc::clone(captures), args.clone(), self);
				let outcome = self.drive_step(&mut tf, None)?;
				self.after_outcome(act, tf, outcome)
			}
		}
	}

	// Fold one async step's outcome: on `Await`, keep the (suspended) frame and
	// run the awaited sub-task; on `Complete`, the frame is gone (its Return
	// already ran cleanups) — run its tail task in its place.
	fn after_outcome(
		&mut self,
		act: &mut Vec<Activation>,
		tf: TaskFrame,
		outcome: StepOutcome,
	) -> Result<Next, RuntimeError> {
		match outcome {
			StepOutcome::Await(sub) => {
				act.push(Activation::Async(tf));
				Ok(Next::Start(sub))
			}
			StepOutcome::Complete(tail) => Ok(Next::Start(tail)),
		}
	}

	// A sub-task produced `v`. Walk down the activation chain: transform through
	// combinator frames, resume the first suspended async frame, or finish.
	fn settle_ok(&mut self, act: &mut Vec<Activation>, mut v: Value) -> Result<Next, RuntimeError> {
		loop {
			match act.pop() {
				None => return Ok(Next::Done(v)),
				Some(Activation::Async(mut tf)) => {
					let outcome = self.drive_step(&mut tf, Some(v))?;
					return self.after_outcome(act, tf, outcome);
				}
				// `then`: feed the value to the continuation, run its task.
				Some(Activation::Then(k)) => {
					let t = self.call_function(k, vec![v])?;
					return Ok(Next::Start(t));
				}
				// `or-else`: success passes straight through; recovery discarded.
				Some(Activation::OrElse(_)) => continue,
				// `attempt`: success reifies to `ok v`, then keeps settling.
				Some(Activation::Attempt) => v = make_result(true, v),
				// `map`: apply the pure function, then keep settling.
				Some(Activation::Map(f)) => v = self.call_function(f, vec![v])?,
			}
		}
	}

	// A sub-task failed with `err`. Walk down: run each async frame's `defer`
	// cleanups (LIFO) and propagate, recover at an `or-else`, or reify at an
	// `attempt`. Reaching the bottom aborts the program.
	fn settle_err(&mut self, act: &mut Vec<Activation>, err: Value) -> Result<Next, RuntimeError> {
		loop {
			match act.pop() {
				None => return Err(RuntimeError::user_abort(format!("{}", err))),
				Some(Activation::Async(mut tf)) => {
					// The awaiting function fails too: run its defers, then keep
					// propagating. (Best-effort; a cleanup that hard-errors
					// bubbles up — refined when cancellation lands in M6.)
					let cleanups = std::mem::take(&mut tf.cleanups);
					for thunk in cleanups.into_iter().rev() {
						self.call_function(thunk, Vec::new())?;
					}
				}
				// `then` / `map`: failure skips the success path; propagate.
				Some(Activation::Then(_)) | Some(Activation::Map(_)) => continue,
				// `or-else`: run the recovery thunk and run the task it returns.
				Some(Activation::OrElse(recover)) => {
					let t = self.call_function(recover, vec![Value::Nothing])?;
					return Ok(Next::Start(t));
				}
				// `attempt`: reify the failure to `err e`, then settle as success.
				Some(Activation::Attempt) => return self.settle_ok(act, make_result(false, err)),
			}
		}
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

// Build a prelude `result` value — `ok v` or `err v` — for `task.attempt`,
// which reifies a task's success/failure into the value channel.
fn make_result(ok: bool, v: Value) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.result".to_string()),
		variant: Rc::new(if ok { "ok" } else { "err" }.to_string()),
		payload: vec![v],
	}))
}
