// The task driver: Pluma's async runtime — a cooperative single-threaded
// scheduler over the CPS state machine.
//
// An async-bearing function (one whose body awaits a task via `try`) is
// compiled to a *step function* — its body lowered with an `Await` instruction
// at each suspension point and a heap-saved frame so it can be resumed. Calling
// such a function builds a cold `Value::Task` recipe (see `do_call`'s `AsyncFn`
// arm); this module *runs* them.
//
// The unit of execution is a **fiber**: one await chain (`Vec<Activation>`)
// belonging to a scope. The scheduler interleaves ready fibers, parks those
// waiting on a timer / another fiber's result / a scope, and drives timers when
// nothing is ready. Suspension never grows the Rust stack: a suspended async
// frame's locals + live temporaries live on the heap in a `TaskFrame`, and the
// await chain lives in the fiber, not in nested `call_function` calls.
//
// `scope` (the keyword) lowers to a `TaskRepr::Scope`; running one creates a
// child scope, runs its body as the scope's root fiber, and blocks the scope's
// completion until every spawned child has settled or been cancelled —
// the structural guarantee. A fail-fast `scope` cancels its siblings when an
// unobserved child fails; a `manual scope` is drained explicitly via `s.next`.

use crate::RuntimeError;
use crate::value::{TaskRepr, Value, VariantData};
use crate::vm::{Frame, VM};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::Instant;

type Fid = usize;
type Sid = usize;
const ROOT_SCOPE: Sid = 0;
const NO_AWAITER: Fid = usize::MAX;

// How a fiber/scope finished. `Cancelled` is structural (out-of-band) — it's
// never delivered to an awaiter as a value; the scope reports it.
#[derive(Clone)]
enum Outcome {
	Ok(Value),
	Err(Value),
	Cancelled,
}

// What a fiber should do on its next turn. `Start` begins running a task value;
// `Ok`/`Err` settle the value (or failure) of whatever the fiber awaited down
// its activation chain.
enum Focus {
	Start(Value),
	Ok(Value),
	Err(Value),
}

// What a fiber is blocked on once it can't make synchronous progress.
enum Wait {
	// Re-ready immediately, behind everything else currently ready.
	Yield,
	// Resume after `ns` nanoseconds (a timer).
	Sleep(i64),
	// Resume when fiber `Fid` settles (awaiting its handle).
	Handle(Fid),
	// Resume when scope `Sid` produces its next child completion (`s.next`).
	Next(Sid),
	// Resume when scope `Sid` finishes (this fiber spawned/entered it).
	Scope(Sid),
}

// The result of advancing a fiber one chunk.
enum Step {
	Done(Outcome),
	Park(Wait),
}

// What stepping one async frame produced before it yielded control back here.
enum StepOutcome {
	// The step function hit an `Await`: this `Value::Task` must run, and its
	// result (or failure) resumes the frame.
	Await(Value),
	// The step function returned. The value is the function's *tail task* —
	// e.g. `task.return v` produces `Pure(v)`. The driver runs it in tail
	// position (the completed frame is gone).
	Complete(Value),
}

// One step toward advancing a fiber: feed a new focus back into the loop, park,
// or finish.
enum Cont {
	Go(Focus),
	Park(Wait),
	Done(Outcome),
}

// A running instance of an async function: a resumable bytecode frame whose
// state lives on the heap so it survives suspension.
struct TaskFrame {
	step_fn: usize,
	captures: Rc<Vec<Value>>,
	// The frame's register window (`nregs` values) saved at the last suspension.
	// On first entry it's the initial registers: the call args padded to `nregs`.
	// Saving the whole window is what lets an `Await` appear mid-expression.
	saved: Vec<Value>,
	resume_ip: usize,
	// The destination register of the `Await` this frame is suspended at; the
	// awaited result is delivered there on resume. `None` before the first await.
	resume_dst: Option<u16>,
	// `defer` cleanups accrued in this frame so far. Held here across
	// suspension; moved into the live VM frame while stepping so PushDefer /
	// Return see them. Run LIFO on normal completion (by the VM's Return), on
	// failure (by the err walk), and on cancellation (by `reap_fiber`).
	cleanups: Vec<Value>,
}

impl TaskFrame {
	fn new(step_fn: usize, captures: Rc<Vec<Value>>, mut args: Vec<Value>, vm: &VM) -> Self {
		let nregs = vm.program.functions[step_fn].nregs as usize;
		args.resize(nregs, Value::Nothing);
		TaskFrame {
			step_fn,
			captures,
			saved: args,
			resume_ip: 0,
			resume_dst: None,
			cleanups: Vec::new(),
		}
	}
}

// A poll-style async-function instance (the `ir::cps` state machine): its poll
// function plus the current heap state value. Advanced by *calling*
// `poll(state, resume)` (`drive_poll`) rather than snapshotting a frame — the
// WASM-shaped alternative to `TaskFrame`. The captures are the closure
// environment (passed on every poll); the state carries the resume tag + the
// vars live across each suspension.
struct PollFrame {
	poll_fn: usize,
	captures: Rc<Vec<Value>>,
	state: Value,
}

// An entry in a fiber's await chain. The top is making progress; each entry
// below is suspended waiting on the one above. `Async`/`Poll` frames are
// *resumed* with a settled value; the combinator frames *transform* it (and are
// popped).
enum Activation {
	// A running async-function instance, suspended at an `Await`.
	Async(TaskFrame),
	// A poll-style async-function instance, suspended between polls.
	Poll(PollFrame),
	// `task.then` continuation: `k : fun a -> task b`, run on success.
	Then(Value),
	// `task.or-else` recovery: `recover : fun nothing -> task a`, on failure.
	OrElse(Value),
	// `task.attempt`: reify the inner outcome into `ok`/`err`.
	Attempt,
	// `task.map`: apply the pure `f : fun a -> b` to a successful value.
	Map(Value),
}

// One cooperatively-scheduled async computation: an await chain plus its
// bookkeeping. A fiber is either ready (queued in `Scheduler::ready`), parked
// (waiting on a timer / handle / scope), or done.
struct Fiber {
	// The await chain. Empty while the fiber is queued or being pumped (it's
	// moved out into a local during `pump`).
	act: Vec<Activation>,
	// The scope this fiber belongs to.
	scope: Sid,
	// `Some(sid)` if this fiber is scope `sid`'s root body fiber.
	runs_scope: Option<Sid>,
	// Fibers blocked awaiting this fiber's handle (`try h`).
	waiters: Vec<Fid>,
	// Settled result, once done. (Currently only read for the root.)
	result: Option<Outcome>,
	// What it's parked on, if parked (so cancellation can cascade into a
	// sub-scope it was awaiting).
	wait: Option<Wait>,
	alive: bool,
}

// A structured-concurrency scope: owns a set of child fibers and can't finish
// until all of them have settled or been cancelled.
struct Scope {
	manual: bool,
	cancelled: bool,
	finalized: bool,
	// The root body fiber (set right after the scope is created).
	body: Fid,
	// Spawned children (via `s.spawn`).
	children: Vec<Fid>,
	// The fiber awaiting this scope's completion (`NO_AWAITER` for the root).
	awaiter: Fid,
	// The body's outcome, once it completes.
	body_done: Option<Outcome>,
	// A failure that fails the whole scope (fail-fast child failure, or a
	// failing body). Takes priority over `body_done` when finalizing.
	failure: Option<Value>,
	// Settled children awaiting an `s.next` drain (manual scopes), FIFO.
	completed: VecDeque<Outcome>,
	// Fibers parked in `s.next` waiting for a completion.
	next_waiters: VecDeque<Fid>,
}

// The scheduler state, owned by the VM for the duration of `run_task` so the
// `scope-*` builtins (which run mid-step, inside a fiber) can reach it.
#[derive(Default)]
pub(crate) struct Scheduler {
	fibers: Vec<Fiber>,
	scopes: Vec<Scope>,
	ready: VecDeque<(Fid, Focus)>,
	// (wake-at ns since `start`, what to do).
	timers: Vec<(i64, Timer)>,
	// Scopes a `scope-cancel` builtin asked to cancel; processed (which runs
	// `defer`s — VM code) at the top of the loop, never mid-step.
	pending_cancels: Vec<Sid>,
	root: Fid,
	root_result: Option<Outcome>,
	start: Option<Instant>,
}

enum Timer {
	Wake(Fid),
	Deadline(Sid),
}

impl Scheduler {
	fn now_ns(&self) -> i64 {
		self.start.map_or(0, |s| s.elapsed().as_nanos() as i64)
	}
}

impl VM {
	// Drive a cold task to completion on this thread. Returns the produced
	// value, or `Err` (a user abort) if the root task failed and nothing
	// recovered it. Called by `run()` when `main` returns a task.
	pub(crate) fn run_task(&mut self, root: Value) -> Result<Value, RuntimeError> {
		self.sched = Scheduler {
			start: Some(Instant::now()),
			..Default::default()
		};
		// The implicit top-level scope (id 0) and the root fiber (id 0).
		self.sched.scopes.push(Scope {
			manual: false,
			cancelled: false,
			finalized: false,
			body: 0,
			children: Vec::new(),
			awaiter: NO_AWAITER,
			body_done: None,
			failure: None,
			completed: VecDeque::new(),
			next_waiters: VecDeque::new(),
		});
		self.sched.fibers.push(Fiber {
			act: Vec::new(),
			scope: ROOT_SCOPE,
			runs_scope: None,
			waiters: Vec::new(),
			result: None,
			wait: None,
			alive: true,
		});
		self.sched.root = 0;
		self.sched.ready.push_back((0, Focus::Start(root)));

		loop {
			// Run any cancellations requested by `s.cancel` during the last step.
			while let Some(sid) = self.sched.pending_cancels.pop() {
				self.cancel_scope(sid)?;
			}
			if let Some(outcome) = &self.sched.root_result {
				return match outcome {
					Outcome::Ok(v) => Ok(v.clone()),
					Outcome::Err(e) => Err(RuntimeError::user_abort(format!("{}", e))),
					Outcome::Cancelled => Ok(Value::Nothing),
				};
			}

			if let Some((fid, focus)) = self.sched.ready.pop_front() {
				if !self.sched.fibers[fid].alive {
					continue; // reaped while it sat in the queue
				}
				self.sched.fibers[fid].wait = None;
				match self.pump(fid, focus)? {
					Step::Done(outcome) => self.fiber_completed(fid, outcome)?,
					Step::Park(wait) => self.park(fid, wait),
				}
			} else if !self.sched.timers.is_empty() {
				self.run_timers();
			} else {
				// Nothing ready and no timers: the root must be done (checked at
				// the top of the next iteration), or we've quiesced.
				if self.sched.root_result.is_none() {
					return Err(RuntimeError::new("VM: async runtime deadlocked"));
				}
			}
		}
	}

	// Advance fiber `fid` from `focus` until it parks or completes. Its await
	// chain is moved into a local for the duration so VM method calls (which
	// take `&mut self`) don't conflict with it.
	fn pump(&mut self, fid: Fid, focus: Focus) -> Result<Step, RuntimeError> {
		let mut act = std::mem::take(&mut self.sched.fibers[fid].act);
		let mut focus = focus;
		loop {
			match self.advance_one(fid, &mut act, focus)? {
				Cont::Go(next) => focus = next,
				Cont::Park(wait) => {
					self.sched.fibers[fid].act = act;
					return Ok(Step::Park(wait));
				}
				Cont::Done(outcome) => return Ok(Step::Done(outcome)),
			}
		}
	}

	// One unit of progress for a fiber: resolve a `Start`ed task, or settle a
	// value/failure down the activation chain.
	fn advance_one(
		&mut self,
		fid: Fid,
		act: &mut Vec<Activation>,
		focus: Focus,
	) -> Result<Cont, RuntimeError> {
		match focus {
			Focus::Start(task) => {
				let repr = match &task {
					Value::Task(r) => Rc::clone(r),
					// A non-task value reaching here is already produced.
					other => return Ok(Cont::Go(Focus::Ok(other.clone()))),
				};
				match repr.as_ref() {
					TaskRepr::Pure(v) => Ok(Cont::Go(Focus::Ok(v.clone()))),
					TaskRepr::Fail(e) => Ok(Cont::Go(Focus::Err(e.clone()))),
					TaskRepr::Yield => Ok(Cont::Park(Wait::Yield)),
					TaskRepr::Sleep(ns) => Ok(Cont::Park(Wait::Sleep(*ns))),
					TaskRepr::Then { task, k } => {
						act.push(Activation::Then(k.clone()));
						Ok(Cont::Go(Focus::Start((**task).clone())))
					}
					TaskRepr::OrElse { task, recover } => {
						act.push(Activation::OrElse(recover.clone()));
						Ok(Cont::Go(Focus::Start((**task).clone())))
					}
					TaskRepr::Attempt { task } => {
						act.push(Activation::Attempt);
						Ok(Cont::Go(Focus::Start((**task).clone())))
					}
					TaskRepr::Map { task, f } => {
						act.push(Activation::Map(f.clone()));
						Ok(Cont::Go(Focus::Start((**task).clone())))
					}
					TaskRepr::Async {
						step_fn,
						captures,
						args,
					} => {
						// Poll-style (`ir::cps`): advance by calling the poll fn. The
						// initial state seeds the params as `__a{i}` (the convention the
						// transform reads). Otherwise: the Await-style frame-snapshot path.
						if let Some(poll_fn) = self.program.async_poll.get(*step_fn).copied().flatten() {
							let mut pf = PollFrame {
								poll_fn: poll_fn as usize,
								captures: Rc::clone(captures),
								state: initial_poll_state(args),
							};
							let outcome = self.drive_poll(&mut pf, Value::Nothing)?;
							Ok(self.after_poll(act, pf, outcome))
						} else {
							let mut tf = TaskFrame::new(*step_fn, Rc::clone(captures), args.clone(), self);
							let outcome = self.drive_step(&mut tf, None)?;
							Ok(self.after_outcome(act, tf, outcome))
						}
					}
					TaskRepr::Shielded { task } => {
						// Run `task` to completion inline, in this same pump. Because
						// the scheduler is single-threaded and only reaps fibers
						// between pumps, nothing can cancel us until we return — the
						// region is uninterruptible, and any pending cancellation is
						// observed only after it settles. Feed the result back into
						// the chain like any sub-task.
						let outcome = self.run_shielded(fid, (**task).clone())?;
						Ok(match outcome {
							Outcome::Ok(v) => Cont::Go(Focus::Ok(v)),
							Outcome::Err(e) => Cont::Go(Focus::Err(e)),
							// run_shielded never yields a cancelled outcome (no reaping
							// happens mid-pump), but stay total.
							Outcome::Cancelled => Cont::Go(Focus::Err(cancelled_error())),
						})
					}
					TaskRepr::Scope { manual, body_fn } => self.start_scope(fid, *manual, body_fn.clone()),
					TaskRepr::Handle(child) => {
						let child = *child;
						match self.sched.fibers[child].result.clone() {
							Some(Outcome::Ok(v)) => Ok(Cont::Go(Focus::Ok(v))),
							Some(Outcome::Err(e)) => Ok(Cont::Go(Focus::Err(e))),
							// A cancelled child awaited directly: treat as a no-value
							// completion (the scope reports the cancellation).
							Some(Outcome::Cancelled) => Ok(Cont::Go(Focus::Ok(Value::Nothing))),
							None => Ok(Cont::Park(Wait::Handle(child))),
						}
					}
					TaskRepr::Next(sid) => Ok(self.drain_next(*sid)),
				}
			}
			Focus::Ok(mut v) => loop {
				match act.pop() {
					None => return Ok(Cont::Done(Outcome::Ok(v))),
					Some(Activation::Async(mut tf)) => {
						let outcome = self.drive_step(&mut tf, Some(v))?;
						return Ok(self.after_outcome(act, tf, outcome));
					}
					Some(Activation::Poll(mut pf)) => {
						let outcome = self.drive_poll(&mut pf, v)?;
						return Ok(self.after_poll(act, pf, outcome));
					}
					Some(Activation::Then(k)) => {
						let t = self.call_function(k, vec![v])?;
						return Ok(Cont::Go(Focus::Start(t)));
					}
					Some(Activation::OrElse(_)) => continue,
					Some(Activation::Attempt) => v = make_result(true, v),
					Some(Activation::Map(f)) => v = self.call_function(f, vec![v])?,
				}
			},
			Focus::Err(e) => loop {
				match act.pop() {
					None => return Ok(Cont::Done(Outcome::Err(e))),
					Some(Activation::Async(tf)) => {
						// The awaiting function fails too: run its defers, then
						// keep propagating.
						for thunk in tf.cleanups.into_iter().rev() {
							self.call_function(thunk, Vec::new())?;
						}
						return Ok(Cont::Go(Focus::Err(e)));
					}
					Some(Activation::Poll(pf)) => {
						// The awaiting poll-style function fails too: run its `defer`
						// cleanups (carried in the suspended state), then keep
						// propagating — mirroring the `Async` arm above.
						self.run_poll_defers(&pf.state)?;
						return Ok(Cont::Go(Focus::Err(e)));
					}
					Some(Activation::Then(_)) | Some(Activation::Map(_)) => continue,
					Some(Activation::OrElse(recover)) => {
						let t = self.call_function(recover, vec![Value::Nothing])?;
						return Ok(Cont::Go(Focus::Start(t)));
					}
					Some(Activation::Attempt) => return Ok(Cont::Go(Focus::Ok(make_result(false, e)))),
				}
			},
		}
	}

	// Fold one async step's outcome into a `Cont`: on `Await`, keep the
	// (suspended) frame and run the awaited sub-task; on `Complete`, the frame
	// is gone (its Return already ran cleanups) — run its tail task in its place.
	fn after_outcome(
		&mut self,
		act: &mut Vec<Activation>,
		tf: TaskFrame,
		outcome: StepOutcome,
	) -> Cont {
		match outcome {
			StepOutcome::Await(sub) => {
				act.push(Activation::Async(tf));
				Cont::Go(Focus::Start(sub))
			}
			StepOutcome::Complete(tail) => Cont::Go(Focus::Start(tail)),
		}
	}

	// The poll-style analogue of `after_outcome`: on `Await` keep the poll frame
	// (with its updated state) and run the awaited sub-task; on `Complete` the
	// machine returned `ready` — run its tail task in its place.
	fn after_poll(&mut self, act: &mut Vec<Activation>, pf: PollFrame, outcome: StepOutcome) -> Cont {
		match outcome {
			StepOutcome::Await(sub) => {
				act.push(Activation::Poll(pf));
				Cont::Go(Focus::Start(sub))
			}
			StepOutcome::Complete(tail) => Cont::Go(Focus::Start(tail)),
		}
	}

	// Advance a poll-style async frame by one step: call `poll(state, resume)`
	// and interpret the returned `__poll` signal. `ready(v)` completes the frame
	// (`v` is its tail task — same contract as a step fn's Return); `pending(sub,
	// state')` suspends — `state'` becomes the frame's state and `sub` is the task
	// to await. Unlike `drive_step`, nothing snapshots the operand stack: the poll
	// fn runs synchronously to its return (it contains no `Await`), and all live
	// state rides in the `state` value. This is the WASM-shaped driver — the CPS
	// pass (`ir::cps`) generates the poll fn; `tests/cps.rs` anchors it to
	// byte-identical behavior vs the Await-style driver.
	//
	// A defer-bearing poll fn returns `ready(value, defers)` — the carried list
	// is run LIFO here (before the tail starts), mirroring the Await-style
	// driver's cleanup run during the step fn's `Return`.
	fn drive_poll(&mut self, pf: &mut PollFrame, resume: Value) -> Result<StepOutcome, RuntimeError> {
		let depth = self.frames.len();
		self.push_frame_with_args(
			pf.poll_fn as u32,
			Rc::clone(&pf.captures),
			vec![pf.state.clone(), resume],
		)?;
		self.run_until_frame_depth(depth)?;
		let result = self
			.pop_stack()
			.ok_or_else(|| RuntimeError::new("VM: poll function returned with empty stack"))?;
		match result {
			Value::Variant(vd) => match vd.variant.as_str() {
				"ready" => {
					let value = vd.payload[0].clone();
					if let Some(defers) = vd.payload.get(1).cloned() {
						self.run_defer_closures(&defers)?;
					}
					Ok(StepOutcome::Complete(value))
				}
				"pending" => {
					pf.state = vd.payload[1].clone();
					Ok(StepOutcome::Await(vd.payload[0].clone()))
				}
				other => Err(RuntimeError::new(format!(
					"VM: poll function returned an unexpected `__poll` variant `{other}`"
				))),
			},
			other => Err(RuntimeError::new(format!(
				"VM: poll function did not return a `__poll` value (got {other})"
			))),
		}
	}

	// Run a list of zero-arg `defer` cleanup closures LIFO (last-pushed first),
	// for their effect. The list is in push order (the CPS pass appends), so
	// reverse it — matching the Await-style frame's `cleanups.into_iter().rev()`.
	fn run_defer_closures(&mut self, list: &Value) -> Result<(), RuntimeError> {
		if let Value::List(ds) = list {
			let ds = ds.borrow().clone();
			for thunk in ds.iter().rev() {
				self.call_function(thunk.clone(), Vec::new())?;
			}
		}
		Ok(())
	}

	// Run the `defer` cleanups carried in a suspended poll frame's state (the
	// `__defers` field — the name is the CPS pass's cross-crate contract; see
	// `ir::cps`). A no-op for a defer-free poll fn (no such field). Used on the
	// failure and cancellation paths, mirroring the Await-style frame's
	// `tf.cleanups` runs.
	fn run_poll_defers(&mut self, state: &Value) -> Result<(), RuntimeError> {
		if let Value::Record(m) = state {
			if let Some(list) = m.get("__defers").cloned() {
				self.run_defer_closures(&list)?;
			}
		}
		Ok(())
	}

	// Drive a shielded task to completion within the current pump (so it can't
	// be interrupted by a concurrent cancellation — see `TaskRepr::Shielded`).
	// Reuses `advance_one` for all the chain logic on a private activation
	// stack; the only difference from the scheduler loop is how parks are
	// handled: `yield`/`sleep` are honored inline (a sleep blocks, which is the
	// price of uninterruptibility), but a cross-fiber await (a scope handle,
	// `s.next`, or a nested `scope`) can't run without the scheduler, so it's a
	// runtime error rather than a silent deadlock.
	fn run_shielded(&mut self, fid: Fid, task: Value) -> Result<Outcome, RuntimeError> {
		let mut act: Vec<Activation> = Vec::new();
		let mut focus = Focus::Start(task);
		loop {
			match self.advance_one(fid, &mut act, focus)? {
				Cont::Go(next) => focus = next,
				Cont::Done(outcome) => return Ok(outcome),
				Cont::Park(wait) => match wait {
					Wait::Yield => focus = Focus::Ok(Value::Nothing),
					Wait::Sleep(ns) => {
						if ns > 0 {
							std::thread::sleep(std::time::Duration::from_nanos(ns as u64));
						}
						focus = Focus::Ok(Value::Nothing);
					}
					Wait::Handle(_) | Wait::Next(_) | Wait::Scope(_) => {
						return Err(RuntimeError::new(
							"task.shielded: a shielded task can't await across fibers (a scope handle, s.next, or a nested scope)",
						));
					}
				},
			}
		}
	}

	// Begin running a `scope` task: create the scope, build its body task by
	// calling the body closure with the scope's handle, queue the body as the
	// scope's root fiber, and park the current fiber until the scope finishes.
	fn start_scope(&mut self, fid: Fid, manual: bool, body_fn: Value) -> Result<Cont, RuntimeError> {
		let sid = self.sched.scopes.len();
		self.sched.scopes.push(Scope {
			manual,
			cancelled: false,
			finalized: false,
			body: 0, // set below
			children: Vec::new(),
			awaiter: fid,
			body_done: None,
			failure: None,
			completed: VecDeque::new(),
			next_waiters: VecDeque::new(),
		});

		// Calling the (async or plain) body closure builds a cold task; it does
		// not run yet.
		let body_task = self.call_function(body_fn, vec![Value::ScopeHandle(sid)])?;

		let bf = self.sched.fibers.len();
		self.sched.fibers.push(Fiber {
			act: Vec::new(),
			scope: sid,
			runs_scope: Some(sid),
			waiters: Vec::new(),
			result: None,
			wait: None,
			alive: true,
		});
		self.sched.scopes[sid].body = bf;
		self.sched.ready.push_back((bf, Focus::Start(body_task)));
		Ok(Cont::Park(Wait::Scope(sid)))
	}

	// `s.next`: hand back the next settled child of scope `sid`, or `none` once
	// every child has been drained, or park until a completion arrives.
	fn drain_next(&mut self, sid: Sid) -> Cont {
		if let Some(outcome) = self.sched.scopes[sid].completed.pop_front() {
			return Cont::Go(Focus::Ok(option_some(settled_result(outcome))));
		}
		if self.scope_children_all_done(sid) {
			return Cont::Go(Focus::Ok(option_none()));
		}
		Cont::Park(Wait::Next(sid))
	}

	fn scope_children_all_done(&self, sid: Sid) -> bool {
		self.sched.scopes[sid]
			.children
			.iter()
			.all(|&c| !self.sched.fibers[c].alive)
	}

	// Register a parked fiber against what it's waiting on.
	fn park(&mut self, fid: Fid, wait: Wait) {
		match &wait {
			Wait::Yield => {
				// Re-ready behind everything currently queued.
				self.sched.ready.push_back((fid, Focus::Ok(Value::Nothing)));
				return;
			}
			Wait::Sleep(ns) => {
				let at = self.sched.now_ns() + (*ns).max(0);
				self.sched.timers.push((at, Timer::Wake(fid)));
			}
			Wait::Handle(child) => self.sched.fibers[*child].waiters.push(fid),
			Wait::Next(sid) => self.sched.scopes[*sid].next_waiters.push_back(fid),
			Wait::Scope(_) => {}
		}
		self.sched.fibers[fid].wait = Some(wait);
	}

	// Sleep until the earliest timer, then fire all due timers: wake sleeping
	// fibers and trip scope deadlines.
	fn run_timers(&mut self) {
		let earliest = self.sched.timers.iter().map(|(at, _)| *at).min().unwrap();
		let now = self.sched.now_ns();
		if earliest > now {
			std::thread::sleep(std::time::Duration::from_nanos((earliest - now) as u64));
		}
		let now = self.sched.now_ns();
		let mut due = Vec::new();
		let mut keep = Vec::new();
		for (at, t) in std::mem::take(&mut self.sched.timers) {
			if at <= now {
				due.push((at, t));
			} else {
				keep.push((at, t));
			}
		}
		self.sched.timers = keep;
		// `thread::sleep` overshoots under load, so several timers can come due
		// in one batch. Fire them in deadline order — never the order they were
		// parked — so the wake order reflects the intended sleep durations
		// (otherwise concurrent combinators like `pool`/`race` would settle
		// their children in a nondeterministic order). Stable sort keeps
		// equal-deadline timers in park order.
		due.sort_by_key(|(at, _)| *at);
		for (_, t) in due {
			match t {
				Timer::Wake(fid) => {
					if self.sched.fibers[fid].alive {
						self.sched.fibers[fid].wait = None;
						self.sched.ready.push_back((fid, Focus::Ok(Value::Nothing)));
					}
				}
				// A deadline fires like `s.cancel`; defer the actual reaping to
				// the loop (it runs `defer`s).
				Timer::Deadline(sid) => self.sched.pending_cancels.push(sid),
			}
		}
	}

	// A fiber finished. Route its outcome: the root sets the program result; a
	// scope body finalizes (or fails) its scope; a spawned child wakes its
	// waiters / feeds `s.next` and may trip fail-fast.
	fn fiber_completed(&mut self, fid: Fid, outcome: Outcome) -> Result<(), RuntimeError> {
		self.sched.fibers[fid].alive = false;
		self.sched.fibers[fid].result = Some(outcome.clone());

		if fid == self.sched.root {
			self.sched.root_result = Some(outcome);
			return Ok(());
		}
		if let Some(sid) = self.sched.fibers[fid].runs_scope {
			return self.on_body_done(sid, outcome);
		}
		let sid = self.sched.fibers[fid].scope;
		self.on_child_done(sid, fid, outcome)
	}

	// The scope's root body finished. Record it, cancel any children that are
	// still running (the structural guarantee), wake any idle `s.next`, then
	// try to finalize.
	fn on_body_done(&mut self, sid: Sid, outcome: Outcome) -> Result<(), RuntimeError> {
		self.sched.scopes[sid].body_done = Some(outcome.clone());
		if let Outcome::Err(e) = &outcome {
			if !self.sched.scopes[sid].manual && self.sched.scopes[sid].failure.is_none() {
				self.sched.scopes[sid].failure = Some(e.clone());
			}
		}
		let children = self.sched.scopes[sid].children.clone();
		for child in children {
			if self.sched.fibers[child].alive {
				self.reap_fiber(child)?;
			}
		}
		// No more completions will arrive — release idle drainers with `none`.
		let waiters = std::mem::take(&mut self.sched.scopes[sid].next_waiters);
		for w in waiters {
			self.sched.ready.push_back((w, Focus::Ok(option_none())));
		}
		self.try_finalize_scope(sid)
	}

	// A spawned child finished. Deliver to its awaiters, record it for `s.next`,
	// trip fail-fast on an unobserved failure, then try to finalize.
	fn on_child_done(&mut self, sid: Sid, fid: Fid, outcome: Outcome) -> Result<(), RuntimeError> {
		let waiters = std::mem::take(&mut self.sched.fibers[fid].waiters);
		let observed = !waiters.is_empty();
		for w in waiters {
			let focus = match &outcome {
				Outcome::Ok(v) => Focus::Ok(v.clone()),
				Outcome::Err(e) => Focus::Err(e.clone()),
				Outcome::Cancelled => Focus::Ok(Value::Nothing),
			};
			self.sched.ready.push_back((w, focus));
		}

		// Feed `s.next`: hand straight to a parked drainer, else queue it.
		if let Some(w) = self.sched.scopes[sid].next_waiters.pop_front() {
			self
				.sched
				.ready
				.push_back((w, Focus::Ok(option_some(settled_result(outcome.clone())))));
		} else {
			self.sched.scopes[sid].completed.push_back(outcome.clone());
		}

		// Fail-fast: an unobserved failure in a non-manual scope cancels it.
		if let Outcome::Err(e) = &outcome {
			let sc = &self.sched.scopes[sid];
			if !observed && !sc.manual && !sc.cancelled && sc.failure.is_none() {
				self.sched.scopes[sid].failure = Some(e.clone());
				return self.cancel_scope(sid);
			}
		}
		self.try_finalize_scope(sid)
	}

	// Cancel a scope and everything it owns: reap the body and all live
	// children (running their `defer`s), which cascades into sub-scopes.
	fn cancel_scope(&mut self, sid: Sid) -> Result<(), RuntimeError> {
		if self.sched.scopes[sid].cancelled || self.sched.scopes[sid].finalized {
			return Ok(());
		}
		self.sched.scopes[sid].cancelled = true;

		let body = self.sched.scopes[sid].body;
		if self.sched.fibers[body].alive {
			self.reap_fiber(body)?;
			if self.sched.scopes[sid].body_done.is_none() {
				self.sched.scopes[sid].body_done = Some(Outcome::Cancelled);
			}
		}
		let children = self.sched.scopes[sid].children.clone();
		for child in children {
			if self.sched.fibers[child].alive {
				self.reap_fiber(child)?;
			}
		}
		self.try_finalize_scope(sid)
	}

	// Abandon a parked (or queued) fiber: cascade into any sub-scope it was
	// awaiting, run its `defer` cleanups (innermost frame first, LIFO within a
	// frame), and mark it cancelled.
	fn reap_fiber(&mut self, fid: Fid) -> Result<(), RuntimeError> {
		if !self.sched.fibers[fid].alive {
			return Ok(());
		}
		self.sched.fibers[fid].alive = false;
		self.sched.fibers[fid].result = Some(Outcome::Cancelled);

		if let Some(Wait::Scope(sub)) = self.sched.fibers[fid].wait.take() {
			self.cancel_scope(sub)?;
		}
		let act = std::mem::take(&mut self.sched.fibers[fid].act);
		for activation in act.into_iter().rev() {
			match activation {
				Activation::Async(tf) => {
					for thunk in tf.cleanups.into_iter().rev() {
						self.call_function(thunk, Vec::new())?;
					}
				}
				Activation::Poll(pf) => self.run_poll_defers(&pf.state)?,
				_ => {}
			}
		}
		Ok(())
	}

	// Finalize a scope once its body is done and every child has settled: wake
	// the awaiter with the scope's result (a fail-fast failure wins over the
	// body's own value).
	fn try_finalize_scope(&mut self, sid: Sid) -> Result<(), RuntimeError> {
		if self.sched.scopes[sid].finalized {
			return Ok(());
		}
		if self.sched.scopes[sid].body_done.is_none() || !self.scope_children_all_done(sid) {
			return Ok(());
		}
		self.sched.scopes[sid].finalized = true;

		let result = match &self.sched.scopes[sid].failure {
			Some(e) => Outcome::Err(e.clone()),
			None => self.sched.scopes[sid].body_done.clone().unwrap(),
		};
		let awaiter = self.sched.scopes[sid].awaiter;
		if awaiter != NO_AWAITER && self.sched.fibers[awaiter].alive {
			let focus = match result {
				Outcome::Ok(v) => Focus::Ok(v),
				Outcome::Err(e) => Focus::Err(e),
				// The scope was cancelled (deadline / explicit `s.cancel`) with
				// no in-band failure of its own, yet a live external awaiter is
				// still expecting an `a`. Surface it as a recoverable failure
				// (`??` / `try`) rather than fabricate a value of the wrong type.
				// (A parent-cancelled scope's awaiter is itself being reaped, so
				// this branch only fires for self-cancellation.)
				Outcome::Cancelled => Focus::Err(cancelled_error()),
			};
			self.sched.fibers[awaiter].wait = None;
			self.sched.ready.push_back((awaiter, focus));
		}
		Ok(())
	}

	// --- hooks for the `scope-*` builtins (run mid-step, inside a fiber) ---

	// `s.spawn t`: start task `t` as a child of scope `sid`; returns the new
	// fiber id, which the builtin wraps in a `TaskRepr::Handle`.
	pub(crate) fn sched_spawn(&mut self, sid: Sid, task: Value) -> Fid {
		let fid = self.sched.fibers.len();
		self.sched.fibers.push(Fiber {
			act: Vec::new(),
			scope: sid,
			runs_scope: None,
			waiters: Vec::new(),
			result: None,
			wait: None,
			alive: true,
		});
		self.sched.scopes[sid].children.push(fid);
		self.sched.ready.push_back((fid, Focus::Start(task)));
		fid
	}

	// `s.cancel`: request cancellation; the loop performs it (running `defer`s)
	// between steps, never mid-step.
	pub(crate) fn sched_cancel(&mut self, sid: Sid) {
		self.sched.pending_cancels.push(sid);
	}

	// `s.cancel-after d`: arm a deadline timer on scope `sid`.
	pub(crate) fn sched_cancel_after(&mut self, sid: Sid, ns: i64) {
		let at = self.sched.now_ns() + ns.max(0);
		self.sched.timers.push((at, Timer::Deadline(sid)));
	}

	// Push a VM frame backed by `tf`'s saved state, run it until it `Await`s or
	// returns, then snapshot (on await) or tear down (on return).
	fn drive_step(
		&mut self,
		tf: &mut TaskFrame,
		resume_val: Option<Value>,
	) -> Result<StepOutcome, RuntimeError> {
		let step_fn = tf.step_fn;
		let nregs = self.program.functions[step_fn].nregs as usize;
		let target_depth = self.frames.len();
		let base = self.stack.len();
		// Restore the frame's saved register window. Async step fns are left
		// un-coerced (all-boxed), so only the boxed window is snapshotted; the raw
		// window is kept length-synced for any coerced sync calls nested below.
		self.stack.extend(tf.saved.iter().cloned());
		self.raw.resize(self.stack.len(), 0);
		self.frames.push(Frame {
			fn_idx: step_fn as u32,
			ip: tf.resume_ip,
			base,
			nregs: nregs as u16,
			captures: Rc::clone(&tf.captures),
			ret_dst: None,
			cleanups: std::mem::take(&mut tf.cleanups),
		});
		// On resume, deliver the awaited result into the `Await`'s destination
		// register (recorded when we suspended).
		if let Some(v) = resume_val {
			let d = tf
				.resume_dst
				.expect("VM: resume without a recorded await dst");
			self.stack[base + d as usize] = v;
		}

		loop {
			// The step frame returned (`ret_dst: None`): its Return ran cleanups,
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
			// Intercept `Await` (only ever present in a step frame, at this depth
			// — nested sync calls contain none). Everything else runs through the
			// normal step loop, including nested calls/returns.
			if frame.ip < func.body.len() {
				if let crate::reg::Instruction::Await { dst, task } = func.body[frame.ip] {
					self.frames[top].ip += 1;
					let fb = self.frames[top].base;
					let nr = self.frames[top].nregs as usize;
					let awaited = self.stack[fb + task as usize].clone();
					tf.saved = self.stack[fb..fb + nr].to_vec();
					tf.resume_ip = self.frames[top].ip;
					tf.resume_dst = Some(dst);
					let popped = self.frames.pop().unwrap();
					tf.cleanups = popped.cleanups;
					self.stack.truncate(popped.base);
					self.raw.truncate(popped.base);
					return Ok(StepOutcome::Await(awaited));
				}
			}
			self.step()?;
		}
	}
}

// The initial state record for a poll-style async fn: tag 0 plus the call args
// seeded positionally as `__a0..__a{N-1}`. **Must match the field convention the
// CPS pass reads** (`ir::cps`): the driver knows only the arg list (not IR
// VarIds), so it keys by arg index; the transform reads param position `i` from
// `__a{i}` in segment 0. Every later state record is built by the poll fn itself.
fn initial_poll_state(args: &[Value]) -> Value {
	let mut m: HashMap<String, Value> = HashMap::with_capacity(args.len() + 1);
	m.insert("__tag".to_string(), Value::Int(0));
	for (i, a) in args.iter().enumerate() {
		m.insert(format!("__a{i}"), a.clone());
	}
	Value::Record(Rc::new(m))
}

// Build a prelude `result` — `ok v` / `err v` — for `task.attempt` and `s.next`.
fn make_result(ok: bool, v: Value) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.result".to_string()),
		variant: Rc::new(if ok { "ok" } else { "err" }.to_string()),
		payload: vec![v],
	}))
}

// A settled child outcome as the `result` value `s.next` yields. Cancellation
// isn't observable in-band; surface it as `err`-free `ok nothing` is wrong, so
// we never reach here with `Cancelled` (drained children are Ok/Err only).
fn settled_result(outcome: Outcome) -> Value {
	match outcome {
		Outcome::Ok(v) => make_result(true, v),
		Outcome::Err(e) => make_result(false, e),
		Outcome::Cancelled => make_result(true, Value::Nothing),
	}
}

fn option_some(v: Value) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.option".to_string()),
		variant: Rc::new("some".to_string()),
		payload: vec![v],
	}))
}

fn option_none() -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.option".to_string()),
		variant: Rc::new("none".to_string()),
		payload: Vec::new(),
	}))
}

// The failure a self-cancelled scope (deadline / explicit `s.cancel`) hands its
// awaiter. Recoverable via `??` / `try`.
fn cancelled_error() -> Value {
	Value::String(Rc::new("scope cancelled".to_string()))
}
