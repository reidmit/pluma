// The async CPS state-machine pass — the WASM-backend prerequisite that turns
// the VM's stack-snapshot async runtime into an explicit, value-carried state
// machine. See IR.md ("Async CPS state-machine pass").
//
// Today an `is_async` function is a *step function*: the VM's `drive_step`
// (`vm/src/task.rs`) runs its bytecode until the `Await` instruction, snapshots
// the whole operand-stack region to the heap, and restores it on resume. That
// snapshot is interpreter-only — WasmGC has no addressable operand stack to grab
// mid-function — so it can't port to WASM.
//
// This pass rewrites such a function into a **poll function**
// `poll(state, resume) -> __poll` where the suspended state is an ordinary value
// (a record): a resume `__tag` plus the variables live across each suspension.
// The poll fn `Match`es on the tag, runs straight-line to the next `Await`, then
// returns `pending(subtask, state')` or `ready(value)`. The VM's poll-driver
// (`drive_poll`) advances it by *calling* it, no stack snapshot — exactly the
// shape WASM wants (state = a GC struct).
//
// **Rollout is per-function and additive.** The original async function `f` is
// left in place (it still drives `MakeAsyncClosure`/`do_call`, so its callers are
// unchanged); we only generate a sibling poll fn and set `f.poll_fn = Some(it)`.
// The driver dispatches on that. Functions this pass doesn't support stay
// `poll_fn: None` and run the existing Await-style driver — both coexist.
//
// **The transform is a flat dispatch loop over basic blocks** — the structured
// encoding of a CFG, and exactly the shape WASM wants (`Loop`+`Match` →
// `loop`+`br_table`). It flattens the function body's structured control flow
// (`If`/`Switch`/`Match`/`Loop`/`Break`/`Continue`/`Return`/`Await`) into basic
// blocks, splits a block at each `Await` (the suspension points), then emits:
//
//     poll(state, resume):
//         pc = state.__tag
//         loop { match pc {
//             0 => <entry block>;   <terminator>
//             1 => ...
//             _ => <last block>
//         } }
//
// Each block's terminator either **returns** (`ready(v)` for a source `Return`,
// `pending(sub, state')` for an `Await`) or **sets `pc` and falls through** so
// the loop re-dispatches (the structured stand-in for a goto). A source `Loop`
// becomes a back-edge: `Continue` (and a fall-out of the body) sets `pc` to the
// loop header, `Break` to the loop exit — so an `await` inside a `while` splits
// the loop into resume segments, and the liveness fixpoint (which already
// handles cycles) threads the loop-carried vars across each suspension. Within
// one poll call locals persist across `pc` hops, so only vars **live across a
// suspension** ride in the state record (`__v{id}`); a resume block unpacks
// exactly those. This handles awaits nested in control flow (incl. loops) and
// early/nested returns, plus `defer`: the scheduled cleanup closures ride in the
// poll state as a `__defers` list (threaded across each suspension like a live
// var, but under a *fixed* field name so the driver can find it), and are run
// LIFO by the VM poll-driver on completion, failure, and cancellation —
// mirroring the Await-style frame's `cleanups`. See `build_poll_fn` and
// `vm::task::run_poll_defers`.
//
// The pass now covers every control-flow shape lowering produces, so any async
// function is transformed. Inert unless `cps_transform` is run (it isn't on the
// default VM path); validated VM-anchored by `tests/cps.rs`.

use crate::types::*;
use std::collections::{BTreeSet, HashMap, HashSet};

// State-record field names. `__tag` is the resume discriminant. The *initial*
// state is built by the VM poll-driver, which knows only the call args (not IR
// VarIds), so it seeds them positionally as `__a{i}` (`initial_poll_state` in
// `vm/src/task.rs` — these names are the cross-crate contract). The transform
// reads param at position `i` from `__a{i}` in segment 0. Every later state
// record is built here, keyed by VarId as `__v{VarId}` (param VarIds are not
// necessarily `0..N-1`, e.g. constrained functions, so VarId keys can't be
// reconstructed by the driver — only the transform uses them).
const TAG_FIELD: &str = "__tag";
fn var_field(v: VarId) -> String {
	format!("__v{}", v.0)
}
fn arg_field(i: usize) -> String {
	format!("__a{i}")
}

// The fixed state field carrying the live `defer` cleanup list — a list of
// zero-arg closures in push order. Threaded across every suspension like a live
// var, but under this *fixed* name (not `__v{id}`) so the VM poll-driver can
// find it to run cleanups on failure/cancellation (`vm::task::run_poll_defers`
// reads the same name — a cross-crate contract). Run LIFO; on completion the
// poll fn also hands it back as the 2nd payload of `ready(value, defers)`.
const DEFERS_FIELD: &str = "__defers";

// The synthetic 2-variant signal a poll fn returns. Tag 0 = `ready(value)`,
// tag 1 = `pending(subtask, state)`. Registered in `program.enums` so the
// `MakeVariant` emit resolves the variant name; the driver reads it structurally.
const POLL_ENUM: &str = "__poll";
const READY_TAG: u32 = 0;
const PENDING_TAG: u32 = 1;

/// Rewrite every supported `is_async` function into poll form, in place. For each
/// transformed `f`, generates a sibling poll function, appends it to
/// `program.functions`, and points `f.poll_fn` at it. Idempotent (already-`Poll`
/// functions are skipped). Inert on the default VM path — only run by the CPS
/// track / its harness.
pub fn cps_transform(program: &mut IrProgram) {
	let base = program.functions.len();
	let mut new_funcs: Vec<Function> = Vec::new();
	for i in 0..base {
		if !eligible(&program.functions[i]) {
			continue;
		}
		if let Some(poll) = build_poll_fn(&program.functions[i]) {
			let poll_id = FuncId((base + new_funcs.len()) as u32);
			program.functions[i].poll_fn = Some(poll_id);
			new_funcs.push(poll);
		}
	}
	if !new_funcs.is_empty() {
		program.functions.extend(new_funcs);
		program
			.enums
			.entry(POLL_ENUM.to_string())
			.or_insert_with(|| vec![("ready".to_string(), 1), ("pending".to_string(), 2)]);
	}
}

// --------------------------------------------------------------------------
// Eligibility — any async function the flattener can handle: all of
// `If`/`Switch`/`Match`/`Loop`/`Break`/`Continue`/`Return`/`Await`/`PushDefer`.
// That's every shape lowering produces, so an async function is eligible as long
// as it actually awaits.
// --------------------------------------------------------------------------

fn eligible(f: &Function) -> bool {
	if !f.is_async || f.poll_fn.is_some() {
		return false;
	}
	block_has_await(&f.body)
}

fn block_has_defer(b: &Block) -> bool {
	b.0.iter().any(|s| match &s.kind {
		StmtKind::PushDefer(_) | StmtKind::RunDefer(_) => true,
		_ => any_child_block(s, block_has_defer),
	})
}

fn block_has_await(b: &Block) -> bool {
	b.0.iter().any(|s| match &s.kind {
		StmtKind::Let(_, rv) | StmtKind::Discard(rv) => matches!(rv, Rvalue::Await(_)),
		_ => any_child_block(s, block_has_await),
	})
}

/// True if `pred` holds for any nested *child* block of `s` (control-flow arms);
/// `s` itself is not inspected. Centralizes the control-flow recursion shape.
fn any_child_block(s: &Stmt, pred: fn(&Block) -> bool) -> bool {
	match &s.kind {
		StmtKind::If(_, t, e) => pred(t) || pred(e),
		StmtKind::Switch { arms, default, .. } => arms.iter().any(|(_, b)| pred(b)) || pred(default),
		StmtKind::Match { arms, .. } => arms.iter().any(|a| pred(&a.body)),
		StmtKind::Loop(b) => pred(b),
		_ => false,
	}
}

// --------------------------------------------------------------------------
// The transform: flatten to a CFG, compute liveness, emit the dispatch loop.
// --------------------------------------------------------------------------

/// A basic block's exit. Every edge target is a `Bid` into `Cfg::blocks`.
enum Term {
	/// Unconditional fall-through to another block.
	Jump(Bid),
	/// Two-way branch on a boolean (from `If`).
	Branch { cond: Atom, t: Bid, e: Bid },
	/// Multi-way integer branch (from `Switch`).
	Switch {
		scrutinee: Atom,
		arms: Vec<(i64, Bid)>,
		default: Bid,
	},
	/// Pattern dispatch (from `Match`); the chosen arm binds its pattern vars
	/// (in *this* block) before continuing to its target.
	MatchT {
		subject: Atom,
		arms: Vec<(Pattern, Bid)>,
	},
	/// Source `Return` — completes the machine (`ready`).
	Return(Atom),
	/// Source `Await` — suspends (`pending`), resuming at `resume` with the
	/// awaited value bound into `bind`.
	Suspend {
		task: Atom,
		bind: Option<VarId>,
		resume: Bid,
	},
}

type Bid = usize;

struct BB {
	/// Straight-line, non-suspending, non-control statements (`Let`/`Discard`
	/// over non-`Await` rvalues).
	stmts: Vec<Stmt>,
	term: Term,
	/// `Some(bind)` if this block is an await *resume* continuation: re-entered
	/// by the driver after a suspension, so it must reload its live-in vars from
	/// the state and bind the awaited result into `bind` (`Some(None)` for a
	/// discarded await). `None` for the entry block and ordinary fall-throughs,
	/// which keep their live-ins in locals from earlier in the same poll call.
	resume_bind: Option<Option<VarId>>,
}

/// Flattens a structured body into basic blocks. `build` returns the entry
/// `Bid` of the chain that runs a statement slice then jumps to a continuation.
struct Cfg {
	blocks: Vec<BB>,
	/// Active loop targets, innermost last: `(header, exit)`. `Continue` jumps to
	/// the header (re-iterate), `Break` to the exit (the loop's continuation).
	loop_ctx: Vec<(Bid, Bid)>,
}

impl Cfg {
	fn new_block(&mut self, stmts: Vec<Stmt>, term: Term) -> Bid {
		let id = self.blocks.len();
		self.blocks.push(BB {
			stmts,
			term,
			resume_bind: None,
		});
		id
	}

	/// Build a block chain that runs `stmts` then continues to `next`. Splits at
	/// the first `Await` or control-flow statement; awaits become `Suspend`
	/// terminators whose resume block runs the remainder.
	fn build(&mut self, stmts: &[Stmt], next: Bid) -> Bid {
		let mut simple: Vec<Stmt> = Vec::new();
		let mut i = 0;
		while i < stmts.len() {
			match &stmts[i].kind {
				StmtKind::Let(_, rv) | StmtKind::Discard(rv) if !matches!(rv, Rvalue::Await(_)) => {
					simple.push(stmts[i].clone());
					i += 1;
				}
				// `PushDefer` is straight-line (it neither suspends nor branches);
				// `build_poll_fn` rewrites it to an append into the `__defers` var.
				StmtKind::PushDefer(_) => {
					simple.push(stmts[i].clone());
					i += 1;
				}
				_ => break,
			}
		}
		if i == stmts.len() {
			return self.new_block(simple, Term::Jump(next));
		}
		let s = &stmts[i];
		let rest = &stmts[i + 1..];
		match &s.kind {
			StmtKind::Let(v, Rvalue::Await(task)) => {
				let resume = self.build(rest, next);
				self.blocks[resume].resume_bind = Some(Some(*v));
				self.new_block(
					simple,
					Term::Suspend {
						task: task.clone(),
						bind: Some(*v),
						resume,
					},
				)
			}
			StmtKind::Discard(Rvalue::Await(task)) => {
				let resume = self.build(rest, next);
				self.blocks[resume].resume_bind = Some(None);
				self.new_block(
					simple,
					Term::Suspend {
						task: task.clone(),
						bind: None,
						resume,
					},
				)
			}
			// A `Return` ends the chain; anything after it is dead.
			StmtKind::Return(a) => self.new_block(simple, Term::Return(a.clone())),
			StmtKind::If(cond, t, e) => {
				let join = self.build(rest, next);
				let t = self.build(&t.0, join);
				let e = self.build(&e.0, join);
				self.new_block(
					simple,
					Term::Branch {
						cond: cond.clone(),
						t,
						e,
					},
				)
			}
			StmtKind::Switch {
				scrutinee,
				arms,
				default,
			} => {
				let join = self.build(rest, next);
				let arms = arms
					.iter()
					.map(|(k, b)| (*k, self.build(&b.0, join)))
					.collect();
				let default = self.build(&default.0, join);
				self.new_block(
					simple,
					Term::Switch {
						scrutinee: scrutinee.clone(),
						arms,
						default,
					},
				)
			}
			StmtKind::Match { subject, arms } => {
				let join = self.build(rest, next);
				let arms = arms
					.iter()
					.map(|a| (a.pattern.clone(), self.build(&a.body.0, join)))
					.collect();
				self.new_block(
					simple,
					Term::MatchT {
						subject: subject.clone(),
						arms,
					},
				)
			}
			StmtKind::Loop(loop_body) => {
				// `next` is the loop's exit (where `Break` and a normal fall-out of
				// the loop land). The header is a forwarding block whose target is the
				// loop body's entry; it exists before the body is built so `Continue`
				// (and the body's fall-through) have a stable back-edge to jump to,
				// then is patched once the body's entry id is known.
				let exit = self.build(rest, next);
				let header = self.new_block(Vec::new(), Term::Jump(exit));
				self.loop_ctx.push((header, exit));
				let body_entry = self.build(&loop_body.0, header);
				self.loop_ctx.pop();
				self.blocks[header].term = Term::Jump(body_entry);
				self.new_block(simple, Term::Jump(header))
			}
			StmtKind::Break => {
				let (_, exit) = *self.loop_ctx.last().expect("cps: `break` outside a loop");
				self.new_block(simple, Term::Jump(exit))
			}
			StmtKind::Continue => {
				let (header, _) = *self
					.loop_ctx
					.last()
					.expect("cps: `continue` outside a loop");
				self.new_block(simple, Term::Jump(header))
			}
			// `RunDefer` is never emitted by lowering (the VM's `Return` runs
			// cleanups); `PushDefer`/`Await`/`Let`/`Discard` were handled above.
			_ => unreachable!("cps: unexpected statement in flattener: {:?}", s.kind),
		}
	}
}

fn term_successors(t: &Term) -> Vec<Bid> {
	match t {
		Term::Jump(b) | Term::Suspend { resume: b, .. } => vec![*b],
		Term::Branch { t, e, .. } => vec![*t, *e],
		Term::Switch { arms, default, .. } => arms
			.iter()
			.map(|(_, b)| *b)
			.chain(std::iter::once(*default))
			.collect(),
		Term::MatchT { arms, .. } => arms.iter().map(|(_, b)| *b).collect(),
		Term::Return(_) => vec![],
	}
}

fn remap_term(t: &Term, m: &HashMap<Bid, Bid>) -> Term {
	let r = |b: &Bid| m[b];
	match t {
		Term::Jump(b) => Term::Jump(r(b)),
		Term::Branch { cond, t, e } => Term::Branch {
			cond: cond.clone(),
			t: r(t),
			e: r(e),
		},
		Term::Switch {
			scrutinee,
			arms,
			default,
		} => Term::Switch {
			scrutinee: scrutinee.clone(),
			arms: arms.iter().map(|(k, b)| (*k, r(b))).collect(),
			default: r(default),
		},
		Term::MatchT { subject, arms } => Term::MatchT {
			subject: subject.clone(),
			arms: arms.iter().map(|(p, b)| (p.clone(), r(b))).collect(),
		},
		Term::Return(a) => Term::Return(a.clone()),
		Term::Suspend { task, bind, resume } => Term::Suspend {
			task: task.clone(),
			bind: *bind,
			resume: r(resume),
		},
	}
}

/// Renumber reachable blocks into a dense `0..k` with `entry` mapped to `0`
/// (the driver seeds the initial `__tag` as `0`), dropping any unreachable
/// block. DFS discovery order is irrelevant beyond `entry` landing first.
fn renumber(blocks: &[BB], entry: Bid) -> Vec<BB> {
	let mut map: HashMap<Bid, Bid> = HashMap::new();
	let mut order: Vec<Bid> = Vec::new();
	let mut stack = vec![entry];
	while let Some(b) = stack.pop() {
		if map.contains_key(&b) {
			continue;
		}
		map.insert(b, order.len());
		order.push(b);
		for s in term_successors(&blocks[b].term) {
			stack.push(s);
		}
	}
	order
		.iter()
		.map(|&old| BB {
			stmts: blocks[old].stmts.clone(),
			term: remap_term(&blocks[old].term, &map),
			resume_bind: blocks[old].resume_bind,
		})
		.collect()
}

/// Reads of an rvalue, as a set of `VarId.0`.
fn reads_of_rvalue(rv: &Rvalue) -> HashSet<u32> {
	let mut s = HashSet::new();
	collect_rvalue_reads(rv, &mut s);
	s
}

/// Per-block live-in sets (`VarId.0`), by backward dataflow to a fixpoint over
/// the (acyclic) block graph. Captures are excluded throughout — they live in
/// the poll fn's capture env, not the state record, so they're never packed.
/// A block's `def` is exactly what it binds (resume var, `Let`s, and the
/// pattern vars its `MatchT` terminator binds in place); its `gen` is the
/// upward-exposed reads.
fn liveness(blocks: &[BB], captures: &HashSet<u32>) -> Vec<BTreeSet<u32>> {
	let n = blocks.len();
	let mut gen: Vec<HashSet<u32>> = Vec::with_capacity(n);
	let mut kill: Vec<HashSet<u32>> = Vec::with_capacity(n);
	for bb in blocks {
		let mut killed: HashSet<u32> = HashSet::new();
		let mut g: HashSet<u32> = HashSet::new();
		let read = |killed: &HashSet<u32>, g: &mut HashSet<u32>, rds: HashSet<u32>| {
			for x in rds {
				if !killed.contains(&x) && !captures.contains(&x) {
					g.insert(x);
				}
			}
		};
		if let Some(Some(v)) = bb.resume_bind {
			killed.insert(v.0); // bound from `resume` before any stmt runs
		}
		for s in &bb.stmts {
			match &s.kind {
				StmtKind::Let(v, rv) => {
					read(&killed, &mut g, reads_of_rvalue(rv));
					killed.insert(v.0);
				}
				StmtKind::Discard(rv) => read(&killed, &mut g, reads_of_rvalue(rv)),
				StmtKind::PushDefer(a) => {
					let mut reads = HashSet::new();
					collect_atom(a, &mut reads);
					read(&killed, &mut g, reads);
				}
				_ => {}
			}
		}
		let mut term_reads = HashSet::new();
		match &bb.term {
			Term::Jump(_) | Term::Return(_) => {}
			Term::Branch { cond, .. } => collect_atom(cond, &mut term_reads),
			Term::Switch { scrutinee, .. } => collect_atom(scrutinee, &mut term_reads),
			Term::MatchT { subject, .. } => collect_atom(subject, &mut term_reads),
			Term::Suspend { task, .. } => collect_atom(task, &mut term_reads),
		}
		if let Term::Return(a) = &bb.term {
			collect_atom(a, &mut term_reads);
		}
		read(&killed, &mut g, term_reads);
		// `MatchT` binds its arms' pattern vars in this block.
		if let Term::MatchT { arms, .. } = &bb.term {
			for (p, _) in arms {
				collect_pattern_binds(p, &mut killed);
			}
		}
		gen.push(g);
		kill.push(killed);
	}

	let mut live_in: Vec<HashSet<u32>> = vec![HashSet::new(); n];
	loop {
		let mut changed = false;
		for b in (0..n).rev() {
			let mut out: HashSet<u32> = HashSet::new();
			for s in term_successors(&blocks[b].term) {
				out.extend(live_in[s].iter().copied());
			}
			let mut in_ = gen[b].clone();
			for v in out {
				if !kill[b].contains(&v) {
					in_.insert(v);
				}
			}
			if in_ != live_in[b] {
				live_in[b] = in_;
				changed = true;
			}
		}
		if !changed {
			break;
		}
	}
	live_in
		.into_iter()
		.map(|s| s.into_iter().collect())
		.collect()
}

fn build_poll_fn(f: &Function) -> Option<Function> {
	if !block_has_await(&f.body) {
		return None;
	}
	// Flatten to a CFG, then renumber so the entry block is id 0.
	let mut cfg = Cfg {
		blocks: Vec::new(),
		loop_ctx: Vec::new(),
	};
	let trap = cfg.new_block(Vec::new(), Term::Return(Atom::Const(Const::Unit)));
	let entry = cfg.build(&f.body.0, trap);
	let blocks = renumber(&cfg.blocks, entry);
	let n = blocks.len();

	let captures: HashSet<u32> = f.captures.iter().map(|v| v.0).collect();
	let live_in = liveness(&blocks, &captures);

	// Fresh VarIds above the function's max; the original body keeps its own.
	let mut next = max_var(f) + 1;
	let mut fresh = || {
		let v = VarId(next);
		next += 1;
		v
	};
	let state_var = fresh();
	let resume_var = fresh();
	let pc_var = fresh();
	// A `defer`-bearing function threads its live cleanup list through the state
	// under the fixed `__defers` field. Defer-free functions skip all of it and
	// keep the arity-1 `ready(value)` contract byte-for-byte unchanged.
	let defers_var = if block_has_defer(&f.body) {
		Some(fresh())
	} else {
		None
	};

	let mut arms: Vec<MatchArm> = Vec::with_capacity(n);
	for (b, bb) in blocks.iter().enumerate() {
		let mut out: Vec<Stmt> = Vec::new();

		// Prologue: the entry block seeds params from the driver's `__a{i}`; a
		// resume block reloads its live-ins from `__v{id}` and the awaited result.
		if b == 0 {
			for (i, p) in f.params.iter().enumerate() {
				out.push(Stmt::synthetic(StmtKind::Let(
					*p,
					Rvalue::GetField(Atom::Var(state_var), arg_field(i), None),
				)));
			}
			// Start the cleanup list empty; appended to by each `PushDefer`.
			if let Some(dv) = defers_var {
				out.push(Stmt::synthetic(StmtKind::Let(
					dv,
					Rvalue::MakeList(Vec::new()),
				)));
			}
		} else if let Some(rb) = bb.resume_bind {
			if let Some(v) = rb {
				out.push(Stmt::synthetic(StmtKind::Let(
					v,
					Rvalue::Use(Atom::Var(resume_var)),
				)));
			}
			for &v in &live_in[b] {
				out.push(Stmt::synthetic(StmtKind::Let(
					VarId(v),
					Rvalue::GetField(Atom::Var(state_var), var_field(VarId(v)), None),
				)));
			}
			// Reload the cleanup list (carried under the fixed field, not `__v{id}`).
			if let Some(dv) = defers_var {
				out.push(Stmt::synthetic(StmtKind::Let(
					dv,
					Rvalue::GetField(Atom::Var(state_var), DEFERS_FIELD.to_string(), None),
				)));
			}
		}

		// Copy the block's straight-line statements, rewriting each `PushDefer`
		// into an append onto the `__defers` accumulator. The poll fn must not use
		// the VM frame's own cleanup stack (that fires at every poll's `Return`,
		// not at the machine's logical exit) — the driver runs `__defers` instead.
		for s in &bb.stmts {
			match (&s.kind, defers_var) {
				(StmtKind::PushDefer(closure), Some(dv)) => {
					out.push(Stmt::new(
						StmtKind::Let(
							dv,
							Rvalue::MakeList(vec![
								ListItem::Spread(Atom::Var(dv)),
								ListItem::Elem(closure.clone()),
							]),
						),
						s.range,
					));
				}
				_ => out.push(s.clone()),
			}
		}

		// Terminator: return (ready/pending) or set `pc` and fall through.
		match &bb.term {
			Term::Return(a) => {
				let rv = fresh();
				// `ready(value)`, or `ready(value, defers)` for a defer-bearing fn —
				// the driver runs the carried list LIFO before completing.
				let mut payload = vec![a.clone()];
				if let Some(dv) = defers_var {
					payload.push(Atom::Var(dv));
				}
				out.push(Stmt::synthetic(StmtKind::Let(
					rv,
					Rvalue::MakeVariant {
						enum_name: POLL_ENUM.to_string(),
						tag: READY_TAG,
						payload,
					},
				)));
				out.push(Stmt::synthetic(StmtKind::Return(Atom::Var(rv))));
			}
			Term::Suspend { task, bind, resume } => {
				let mut fields = vec![(
					TAG_FIELD.to_string(),
					Atom::Const(Const::Int(*resume as i64)),
				)];
				for &v in &live_in[*resume] {
					if Some(VarId(v)) == *bind {
						continue; // supplied by `resume`, not the state
					}
					fields.push((var_field(VarId(v)), Atom::Var(VarId(v))));
				}
				// Carry the cleanup list across the suspension under its fixed field.
				if let Some(dv) = defers_var {
					fields.push((DEFERS_FIELD.to_string(), Atom::Var(dv)));
				}
				let st = fresh();
				out.push(Stmt::synthetic(StmtKind::Let(
					st,
					Rvalue::MakeRecord(fields),
				)));
				let pv = fresh();
				out.push(Stmt::synthetic(StmtKind::Let(
					pv,
					Rvalue::MakeVariant {
						enum_name: POLL_ENUM.to_string(),
						tag: PENDING_TAG,
						payload: vec![task.clone(), Atom::Var(st)],
					},
				)));
				out.push(Stmt::synthetic(StmtKind::Return(Atom::Var(pv))));
			}
			Term::Jump(t) => out.push(set_pc(pc_var, *t)),
			Term::Branch { cond, t, e } => out.push(Stmt::synthetic(StmtKind::If(
				cond.clone(),
				Block(vec![set_pc(pc_var, *t)]),
				Block(vec![set_pc(pc_var, *e)]),
			))),
			Term::Switch {
				scrutinee,
				arms,
				default,
			} => out.push(Stmt::synthetic(StmtKind::Switch {
				scrutinee: scrutinee.clone(),
				arms: arms
					.iter()
					.map(|(k, t)| (*k, Block(vec![set_pc(pc_var, *t)])))
					.collect(),
				default: Box::new(Block(vec![set_pc(pc_var, *default)])),
			})),
			Term::MatchT { subject, arms } => out.push(Stmt::synthetic(StmtKind::Match {
				subject: subject.clone(),
				arms: arms
					.iter()
					.map(|(p, t)| MatchArm {
						pattern: p.clone(),
						body: Block(vec![set_pc(pc_var, *t)]),
					})
					.collect(),
			})),
		}

		// Dispatch pattern: `Literal(id)` for every block but the last, which is
		// the exhaustive `Wildcard` (so the `Match` always selects an arm).
		let pattern = if b + 1 == n {
			Pattern::Wildcard
		} else {
			Pattern::Literal(Const::Int(b as i64))
		};
		arms.push(MatchArm {
			pattern,
			body: Block(out),
		});
	}

	let body = Block(vec![
		Stmt::synthetic(StmtKind::Let(
			pc_var,
			Rvalue::GetField(Atom::Var(state_var), TAG_FIELD.to_string(), None),
		)),
		Stmt::synthetic(StmtKind::Loop(Block(vec![Stmt::synthetic(
			StmtKind::Match {
				subject: Atom::Var(pc_var),
				arms,
			},
		)]))),
		// Unreachable (the loop only exits via a `Return` inside an arm); a
		// non-`__poll` value here would make the driver fault loudly.
		Stmt::synthetic(StmtKind::Return(Atom::Const(Const::Unit))),
	]);

	Some(Function {
		name: format!("{}@poll", f.name),
		module: f.module.clone(),
		params: vec![state_var, resume_var],
		captures: f.captures.clone(),
		is_async: false,
		poll_fn: None,
		body,
		var_reprs: Vec::new(),
		param_reprs: vec![Repr::Boxed, Repr::Boxed],
		ret_repr: Repr::Boxed,
	})
}

/// `pc := target` — sets the dispatch variable so the enclosing loop re-selects
/// the target block on the next iteration (the structured stand-in for a goto).
fn set_pc(pc: VarId, target: Bid) -> Stmt {
	Stmt::synthetic(StmtKind::Let(
		pc,
		Rvalue::Use(Atom::Const(Const::Int(target as i64))),
	))
}

// --------------------------------------------------------------------------
// Var collection helpers (reads / binds), used for liveness and `max_var`.
// --------------------------------------------------------------------------

fn max_var(f: &Function) -> u32 {
	let mut m = 0u32;
	for v in f.params.iter().chain(f.captures.iter()) {
		m = m.max(v.0);
	}
	let mut s = HashSet::new();
	for st in &f.body.0 {
		collect_stmt_reads(st, &mut s);
		collect_stmt_binds(st, &mut s);
	}
	for v in s {
		m = m.max(v);
	}
	m
}

fn collect_atom(a: &Atom, set: &mut HashSet<u32>) {
	if let Atom::Var(v) = a {
		set.insert(v.0);
	}
}

fn collect_rvalue_reads(rv: &Rvalue, set: &mut HashSet<u32>) {
	use Rvalue::*;
	match rv {
		Use(a)
		| Not(a)
		| Box(a)
		| Unbox(a, _)
		| GetDictMethod(a, _)
		| GetField(a, _, _)
		| GetElement(a, _)
		| Await(a)
		| GetTag(a)
		| GetPayload(a, _) => collect_atom(a, set),
		Bin(_, a, b) => {
			collect_atom(a, set);
			collect_atom(b, set);
		}
		Call(_, args) => args.iter().for_each(|a| collect_atom(a, set)),
		CallClosure(c, args) | TailCall(c, args) => {
			collect_atom(c, set);
			args.iter().for_each(|a| collect_atom(a, set));
		}
		MakeDict(xs) | MakeTuple(xs) | Interpolate(xs) => xs.iter().for_each(|a| collect_atom(a, set)),
		MakeClosure(_, caps) => caps.iter().for_each(|a| collect_atom(a, set)),
		MakeRecord(fields) => fields.iter().for_each(|(_, a)| collect_atom(a, set)),
		RecordUpdate { base, fields } => {
			collect_atom(base, set);
			fields.iter().for_each(|(_, a)| collect_atom(a, set));
		}
		MakeVariant { payload, .. } => payload.iter().for_each(|a| collect_atom(a, set)),
		MakeList(items) => items.iter().for_each(|it| match it {
			ListItem::Elem(a) | ListItem::Spread(a) => collect_atom(a, set),
		}),
		GlobalRef(_) | Builtin(_) | MakeVariantCtor { .. } => {}
	}
}

fn collect_stmt_reads(s: &Stmt, set: &mut HashSet<u32>) {
	match &s.kind {
		StmtKind::Let(_, rv) | StmtKind::Discard(rv) => collect_rvalue_reads(rv, set),
		StmtKind::Return(a) | StmtKind::PushDefer(a) => collect_atom(a, set),
		StmtKind::If(c, t, e) => {
			collect_atom(c, set);
			collect_block_reads(t, set);
			collect_block_reads(e, set);
		}
		StmtKind::Switch {
			scrutinee,
			arms,
			default,
		} => {
			collect_atom(scrutinee, set);
			arms.iter().for_each(|(_, b)| collect_block_reads(b, set));
			collect_block_reads(default, set);
		}
		StmtKind::Match { subject, arms } => {
			collect_atom(subject, set);
			arms.iter().for_each(|a| collect_block_reads(&a.body, set));
		}
		StmtKind::Loop(b) => collect_block_reads(b, set),
		StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
	}
}

fn collect_block_reads(b: &Block, set: &mut HashSet<u32>) {
	b.0.iter().for_each(|s| collect_stmt_reads(s, set));
}

fn collect_pattern_binds(p: &Pattern, set: &mut HashSet<u32>) {
	match p {
		Pattern::Wildcard | Pattern::Literal(_) => {}
		Pattern::Bind(v) => {
			set.insert(v.0);
		}
		Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
			fields.iter().for_each(|f| collect_pattern_binds(f, set))
		}
		Pattern::List { items, rest } => {
			items.iter().for_each(|p| collect_pattern_binds(p, set));
			if let Some(ListRest::Bind(v)) = rest {
				set.insert(v.0);
			}
		}
		Pattern::Record { fields, rest, .. } => {
			fields
				.iter()
				.for_each(|(_, p)| collect_pattern_binds(p, set));
			if let RecordRest::Bind(v) = rest {
				set.insert(v.0);
			}
		}
	}
}

fn collect_stmt_binds(s: &Stmt, set: &mut HashSet<u32>) {
	match &s.kind {
		StmtKind::Let(v, _) => {
			set.insert(v.0);
		}
		StmtKind::If(_, t, e) => {
			collect_block_binds(t, set);
			collect_block_binds(e, set);
		}
		StmtKind::Switch { arms, default, .. } => {
			arms.iter().for_each(|(_, b)| collect_block_binds(b, set));
			collect_block_binds(default, set);
		}
		StmtKind::Match { arms, .. } => arms.iter().for_each(|a| {
			collect_pattern_binds(&a.pattern, set);
			collect_block_binds(&a.body, set);
		}),
		StmtKind::Loop(b) => collect_block_binds(b, set),
		_ => {}
	}
}

fn collect_block_binds(b: &Block, set: &mut HashSet<u32>) {
	b.0.iter().for_each(|s| collect_stmt_binds(s, set));
}
