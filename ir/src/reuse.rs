// Reuse — opportunistic in-place reuse for value-semantics `std/dict`.
//
// `dict.insert` returns a *new* dict; the input is never mutated (value semantics).
// A persistent insert path-copies the root→leaf nodes, so a tight accumulator loop
// (`m = insert m k v` each iteration) reallocates per iteration where the old mutable
// table mutated in place. The transient runtime (`__cnode_tinsert`, an edit token
// compared by `ref.eq`) can mutate nodes in place *when the input dict is uniquely
// owned and dead afterward* — then the mutation is unobservable and value semantics
// are preserved. This pass proves that condition statically (Pluma runs on a tracing
// GC, so the refcount trick Koka/Roc use is unavailable) and
// rewrites the proven-safe `dict.insert`s to the transient `dict-insert-into`,
// threading one fresh owner token through the region.
//
// Scope (v1): the **loop-carried accumulator** that `loopify` produces — a parameter
// threaded single-headedly through a `Loop` and updated by `dict.insert`. That is the
// shape of the tally/`fill` benchmarks and the headline regression. The analysis is
// deliberately conservative: it fires only when it can prove every condition, and
// leaves the persistent insert (a correct copy) in place otherwise. Getting it wrong
// would be silent heap corruption, so "when unsure, copy."
//
// Runs in `wasm::emit`'s pipeline after `resolve_builtins` (so `dict.insert` is a
// `Call(Callee::Builtin("dict-insert", …))`) and `loopify` (so the accumulator is a
// `Loop`), and before the repr pass (so the minted token gets a repr).

use crate::types::*;
use std::collections::HashSet;

/// The reuse pass's verdict on one `dict.insert` site — what `report` yields so the
/// soundness test can assert the pass mutates in place exactly where it's safe to
/// (and copies otherwise), without inspecting the rewritten IR.
pub struct ReuseNote {
	/// The enclosing function's module (e.g. `main`, `std/dict`).
	pub module: String,
	/// True when the insert was rewritten to the in-place transient.
	pub reused: bool,
}

/// Analyze (without rewriting) every `dict.insert` site and report whether the reuse
/// pass would mutate in place or copy. Run on a program already through `loopify`
/// and `resolve_builtins` (so inserts are resolved builtin calls in loop form) —
/// exactly the state `reuse` itself sees.
pub fn report(program: &IrProgram) -> Vec<ReuseNote> {
	let mut notes = Vec::new();
	for f in &program.functions {
		report_fn(f, &mut notes);
	}
	notes
}

fn report_fn(f: &Function, notes: &mut Vec<ReuseNote>) {
	// The loopify shape `Loop { … } ; Return(result)` is the only one the analysis
	// can reuse; classify each insert against it.
	let shape = (f.body.0.len() == 2)
		.then(|| match (&f.body.0[0].kind, &f.body.0[1].kind) {
			(StmtKind::Loop(l), StmtKind::Return(Atom::Var(r))) => Some((l, r.0)),
			_ => None,
		})
		.flatten();
	for d in insert_sites(&f.body) {
		let reused = match shape {
			Some((loop_body, result))
				if assigned_in_block(loop_body).contains(&d) && f.params.iter().any(|p| p.0 == d) =>
			{
				is_eligible(f, d, result, loop_body)
			}
			_ => false,
		};
		notes.push(ReuseNote {
			module: f.module.clone(),
			reused,
		});
	}
}

/// Every `dict.insert` consume site in `b`: the dict var it consumes.
fn insert_sites(b: &Block) -> Vec<u32> {
	let mut out = Vec::new();
	fn walk(b: &Block, out: &mut Vec<u32>) {
		for s in &b.0 {
			if let StmtKind::Let(_, Rvalue::Call(Callee::Builtin(tag, _), args)) = &s.kind {
				if tag == "dict-insert" {
					if let Some(Atom::Var(v)) = args.get(1) {
						out.push(v.0);
					}
				}
			}
			match &s.kind {
				StmtKind::If(_, t, e) => {
					walk(t, out);
					walk(e, out);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, out);
					}
					walk(default, out);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(&arm.body, out);
					}
				}
				StmtKind::Loop(blk) => walk(blk, out),
				_ => {}
			}
		}
	}
	walk(b, &mut out);
	out
}

/// Rewrite proven-safe `dict.insert` accumulators to the transient in-place insert.
pub fn reuse(program: &mut IrProgram) {
	for f in &mut program.functions {
		reuse_fn(f);
	}
}

/// How a dict builtin treats its dict argument, and at which arg index that dict
/// sits (the `where (hash k)` witness occupies index 0 for the keyed ops).
enum DictUse {
	/// Reads the dict, does not retain or mutate it (`lookup`, `size`, `entries`, …).
	Borrow,
	/// Consumes the dict and returns a new one via `dict.insert` — rewritable.
	ConsumeInsert,
	/// Consumes via `remove`/`update` — no transient variant yet, so a dict that
	/// flows through one is left fully persistent (ineligible).
	ConsumeOther,
}

fn dict_arg(tag: &str) -> Option<(usize, DictUse)> {
	match tag {
		"dict-insert" => Some((1, DictUse::ConsumeInsert)),
		"dict-remove" | "dict-update" => Some((1, DictUse::ConsumeOther)),
		"dict-lookup" => Some((1, DictUse::Borrow)),
		"dict-size" | "dict-entries" | "dict-map" | "dict-filter" | "dict-clear" => {
			Some((0, DictUse::Borrow))
		}
		_ => None,
	}
}

fn reuse_fn(f: &mut Function) {
	// Only the `loopify` shape: `Loop { … } ; Return(result)`.
	if f.body.0.len() != 2 {
		return;
	}
	if !matches!(f.body.0[0].kind, StmtKind::Loop(_)) {
		return;
	}
	let StmtKind::Return(Atom::Var(result)) = f.body.0[1].kind else {
		return;
	};

	let StmtKind::Loop(loop_body) = &f.body.0[0].kind else {
		unreachable!()
	};
	// A candidate is a loop-carried parameter (reassigned inside the loop) that is
	// consumed by `dict.insert`. `result` is the var returned after the loop.
	let reassigned = assigned_in_block(loop_body);
	let params = f.params.clone();
	let mut eligible: HashSet<u32> = HashSet::new();
	for d in &params {
		if !reassigned.contains(&d.0) {
			continue;
		}
		if is_eligible(f, d.0, result.0, loop_body) {
			eligible.insert(d.0);
		}
	}
	if eligible.is_empty() {
		return;
	}

	// Commit: thread one fresh owner token through the loop and rewrite every
	// eligible `dict.insert` to the transient `dict-insert-into`. Distinct eligible
	// dicts are independent values (never alias — the escape scan guarantees it), so
	// sharing one token is safe.
	let token = VarId(next_var(f));
	let range = f.body.0[0].range;
	let StmtKind::Loop(loop_body) = &mut f.body.0[0].kind else {
		unreachable!()
	};
	rewrite_block(loop_body, &eligible, token);
	f.body.0.insert(
		0,
		Stmt::new(
			StmtKind::Let(
				token,
				Rvalue::Call(
					Callee::Builtin("dict-mint-token".into(), Repr::Boxed),
					vec![],
				),
			),
			range,
		),
	);
}

fn is_eligible(f: &Function, d: u32, result: u32, loop_body: &Block) -> bool {
	eligibility(f, d, result, loop_body).is_ok()
}

/// Why a loop-carried dict was (not) eligible for in-place reuse. `Ok(())` means
/// safe to reuse; each `Err` is the reason it must stay a persistent copy. The
/// conditions: (a) every use of `d` is a dict borrow, a `dict.insert` consume, or
/// the return path — no escape, no aliasing copy; (b) at least one `dict.insert`
/// consume; (c) each consume is the last use of that `d` value before `d` is
/// reassigned (dead-after); and (d) each consume's *result* flows only into `d`'s
/// reassignment (the freshly-inserted dict is token-owned too, so it must not escape).
fn eligibility(f: &Function, d: u32, result: u32, loop_body: &Block) -> Result<(), &'static str> {
	let mut scan = UseScan {
		d,
		result,
		ok: true,
		inserts: 0,
	};
	scan.block(&f.body);
	if !scan.ok {
		return Err("the dict escapes or is aliased (stored, captured, or passed elsewhere)");
	}
	if scan.inserts == 0 {
		return Err("no `dict.insert` consumes it");
	}
	if !dead_after_ok(loop_body, d) {
		return Err("it is read again after the insert (not dead), so the old value is still needed");
	}
	let reads = read_counts(f);
	if !collect_insert_results(loop_body, d)
		.into_iter()
		.all(|r| result_flows_to_d(f, r, d, &reads))
	{
		return Err("the inserted dict escapes instead of just continuing the accumulator");
	}
	Ok(())
}

/// The `r` of every `Let(r, dict.insert(.., d, ..))` in `b` (recursively).
fn collect_insert_results(b: &Block, d: u32) -> Vec<u32> {
	let mut out = Vec::new();
	fn walk(b: &Block, d: u32, out: &mut Vec<u32>) {
		for s in &b.0 {
			if let StmtKind::Let(r, _) = &s.kind {
				if is_insert_consume(&s.kind, d) {
					out.push(r.0);
				}
			}
			match &s.kind {
				StmtKind::If(_, t, e) => {
					walk(t, d, out);
					walk(e, d, out);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, d, out);
					}
					walk(default, d, out);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(&arm.body, d, out);
					}
				}
				StmtKind::Loop(blk) => walk(blk, d, out),
				_ => {}
			}
		}
	}
	walk(b, d, &mut out);
	out
}

/// `r` is read exactly once, and that read copies it straight into `d` (`Let(d,
/// Use(r))`) or into one intermediate temp that is itself single-use-copied into `d`
/// — the shape `loopify`'s parallel-assignment staging produces. Anything else
/// (multiple reads, a non-copy use, a longer chain) is rejected.
fn result_flows_to_d(
	f: &Function,
	r: u32,
	d: u32,
	reads: &std::collections::HashMap<u32, usize>,
) -> bool {
	match copy_target(f, r, reads) {
		Some(x) if x == d => true,
		Some(x) => copy_target(f, x, reads) == Some(d),
		None => false,
	}
}

/// If `v` is read exactly once and that read is `Let(x, Use(Var(v)))`, return `x`.
fn copy_target(f: &Function, v: u32, reads: &std::collections::HashMap<u32, usize>) -> Option<u32> {
	if reads.get(&v).copied().unwrap_or(0) != 1 {
		return None;
	}
	fn find(b: &Block, v: u32) -> Option<u32> {
		for s in &b.0 {
			if let StmtKind::Let(x, Rvalue::Use(Atom::Var(u))) = &s.kind {
				if u.0 == v {
					return Some(x.0);
				}
			}
			let nested = match &s.kind {
				StmtKind::If(_, t, e) => find(t, v).or_else(|| find(e, v)),
				StmtKind::Switch { arms, default, .. } => arms
					.iter()
					.find_map(|(_, blk)| find(blk, v))
					.or_else(|| find(default, v)),
				StmtKind::Match { arms, .. } => arms.iter().find_map(|arm| find(&arm.body, v)),
				StmtKind::Loop(blk) => find(blk, v),
				_ => None,
			};
			if nested.is_some() {
				return nested;
			}
		}
		None
	}
	find(&f.body, v)
}

/// Read-occurrence count for every variable across the function (a `Let` target is a
/// write, not counted; every `Atom::Var` read is).
fn read_counts(f: &Function) -> std::collections::HashMap<u32, usize> {
	let mut counts = std::collections::HashMap::new();
	fn block(b: &Block, c: &mut std::collections::HashMap<u32, usize>) {
		for s in &b.0 {
			let mut bump = |a: &Atom| {
				if let Atom::Var(v) = a {
					*c.entry(v.0).or_insert(0) += 1;
				}
			};
			match &s.kind {
				StmtKind::Let(_, rv) | StmtKind::Discard(rv) => rvalue_atoms(rv, &mut bump),
				StmtKind::Return(a) | StmtKind::PushDefer(a) => bump(a),
				StmtKind::If(cnd, _, _) => bump(cnd),
				StmtKind::Switch { scrutinee, .. } => bump(scrutinee),
				StmtKind::Match { subject, .. } => bump(subject),
				StmtKind::Loop(_) | StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
			}
			match &s.kind {
				StmtKind::If(_, t, e) => {
					block(t, c);
					block(e, c);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						block(blk, c);
					}
					block(default, c);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						block(&arm.body, c);
					}
				}
				StmtKind::Loop(blk) => block(blk, c),
				_ => {}
			}
		}
	}
	block(&f.body, &mut counts);
	counts
}

// --------------------------------------------------------------------------
// Escape / aliasing scan: classify every *read* of `d` across the function.
// --------------------------------------------------------------------------

struct UseScan {
	d: u32,
	result: u32,
	ok: bool,
	inserts: usize,
}

impl UseScan {
	fn is_d(&self, a: &Atom) -> bool {
		matches!(a, Atom::Var(v) if v.0 == self.d)
	}

	/// A `d` appearing in a position other than a recognized dict op / the return
	/// path means it may be retained or aliased — reject (conservative).
	fn escape_if_d(&mut self, a: &Atom) {
		if self.is_d(a) {
			self.ok = false;
		}
	}

	fn block(&mut self, b: &Block) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, rv) => self.rvalue(rv, v.0 == self.result),
				StmtKind::Discard(rv) => self.rvalue(rv, false),
				// Returning `d` (or the result var that copies it) hands the final dict
				// to the caller; the token is function-local, so the returned value is
				// effectively frozen (persistent ops copy it). Not an escape.
				StmtKind::Return(_) => {}
				StmtKind::PushDefer(a) => self.escape_if_d(a),
				StmtKind::If(c, t, e) => {
					self.escape_if_d(c);
					self.block(t);
					self.block(e);
				}
				StmtKind::Switch {
					scrutinee,
					arms,
					default,
				} => {
					self.escape_if_d(scrutinee);
					for (_, blk) in arms {
						self.block(blk);
					}
					self.block(default);
				}
				StmtKind::Match { subject, arms } => {
					self.escape_if_d(subject);
					for arm in arms {
						self.block(&arm.body);
					}
				}
				StmtKind::Loop(blk) => self.block(blk),
				StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
			}
		}
	}

	/// `is_result_target` is true when this rvalue is the RHS of `Let(result, …)` —
	/// the only place a bare `Use(Var(d))` (the copy `loopify` emits for a returned
	/// accumulator) is allowed.
	fn rvalue(&mut self, rv: &Rvalue, is_result_target: bool) {
		match rv {
			Rvalue::Use(a) => {
				if self.is_d(a) && !is_result_target {
					self.ok = false;
				}
			}
			Rvalue::Call(Callee::Builtin(tag, _), args) => {
				match dict_arg(tag) {
					Some((idx, kind)) => {
						for (i, a) in args.iter().enumerate() {
							if self.is_d(a) {
								if i != idx {
									self.ok = false; // `d` used as key/value/etc.
								} else {
									match kind {
										DictUse::Borrow => {}
										DictUse::ConsumeInsert => self.inserts += 1,
										DictUse::ConsumeOther => self.ok = false,
									}
								}
							}
						}
					}
					None => {
						for a in args {
							self.escape_if_d(a);
						}
					}
				}
			}
			Rvalue::Bin(_, a, b) => {
				self.escape_if_d(a);
				self.escape_if_d(b);
			}
			Rvalue::Not(a) | Rvalue::Box(a) | Rvalue::Unbox(a, _) => self.escape_if_d(a),
			Rvalue::Call(_, args)
			| Rvalue::MakeDict(args)
			| Rvalue::MakeTuple(args)
			| Rvalue::Interpolate(args)
			| Rvalue::MakeClosure(_, args)
			| Rvalue::MakeVariant { payload: args, .. } => {
				for a in args {
					self.escape_if_d(a);
				}
			}
			Rvalue::CallClosure(g, args) | Rvalue::TailCall(g, args) => {
				self.escape_if_d(g);
				for a in args {
					self.escape_if_d(a);
				}
			}
			Rvalue::TailCallDirect(_, args) => {
				for a in args {
					self.escape_if_d(a);
				}
			}
			Rvalue::RecordUpdate { base, fields } => {
				self.escape_if_d(base);
				for (_, a) in fields {
					self.escape_if_d(a);
				}
			}
			Rvalue::MakeRecord(fields) => {
				for (_, a) in fields {
					self.escape_if_d(a);
				}
			}
			Rvalue::GetField(a, _, _)
			| Rvalue::GetElement(a, _)
			| Rvalue::GetTag(a)
			| Rvalue::GetPayload(a, _)
			| Rvalue::GetDictMethod(a, _)
			| Rvalue::Await(a) => self.escape_if_d(a),
			Rvalue::MakeList(items) => {
				for it in items {
					match it {
						ListItem::Elem(a) | ListItem::Spread(a) => self.escape_if_d(a),
					}
				}
			}
			Rvalue::GlobalRef(_) | Rvalue::Builtin(_) | Rvalue::MakeVariantCtor { .. } => {}
		}
	}
}

// --------------------------------------------------------------------------
// Dead-after check: each `dict.insert` consume of `d` must be the last use of
// that value before `d` is reassigned, in the same block.
// --------------------------------------------------------------------------

/// For every block in/under `b`, each `dict.insert` consume of `d` must be followed
/// (in the same block) by a reassignment `Let(d, …)` with no read of `d` in between.
fn dead_after_ok(b: &Block, d: u32) -> bool {
	for (i, s) in b.0.iter().enumerate() {
		if is_insert_consume(&s.kind, d) {
			// Find the reassignment of `d` after this consume; reject any read of `d`
			// before it.
			let mut reassigned = false;
			for later in &b.0[i + 1..] {
				if let StmtKind::Let(v, _) = &later.kind {
					if v.0 == d {
						reassigned = true;
						break;
					}
				}
				if stmt_reads_var(&later.kind, d) {
					return false; // `d` read after the consume, before reassignment
				}
			}
			if !reassigned {
				return false;
			}
		}
		// Recurse into nested control flow.
		match &s.kind {
			StmtKind::If(_, t, e) => {
				if !dead_after_ok(t, d) || !dead_after_ok(e, d) {
					return false;
				}
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					if !dead_after_ok(blk, d) {
						return false;
					}
				}
				if !dead_after_ok(default, d) {
					return false;
				}
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					if !dead_after_ok(&arm.body, d) {
						return false;
					}
				}
			}
			StmtKind::Loop(blk) => {
				if !dead_after_ok(blk, d) {
					return false;
				}
			}
			_ => {}
		}
	}
	true
}

fn is_insert_consume(kind: &StmtKind, d: u32) -> bool {
	matches!(
		kind,
		StmtKind::Let(_, Rvalue::Call(Callee::Builtin(tag, _), args))
			if tag == "dict-insert" && args.get(1).is_some_and(|a| matches!(a, Atom::Var(v) if v.0 == d))
	)
}

// --------------------------------------------------------------------------
// Rewrite: `dict.insert` of an eligible `d` → `dict-insert-into` + token.
// --------------------------------------------------------------------------

fn rewrite_block(b: &mut Block, eligible: &HashSet<u32>, token: VarId) {
	for s in &mut b.0 {
		if let StmtKind::Let(_, rv) = &mut s.kind {
			rewrite_rvalue(rv, eligible, token);
		}
		match &mut s.kind {
			StmtKind::If(_, t, e) => {
				rewrite_block(t, eligible, token);
				rewrite_block(e, eligible, token);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					rewrite_block(blk, eligible, token);
				}
				rewrite_block(default, eligible, token);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					rewrite_block(&mut arm.body, eligible, token);
				}
			}
			StmtKind::Loop(blk) => rewrite_block(blk, eligible, token),
			_ => {}
		}
	}
}

fn rewrite_rvalue(rv: &mut Rvalue, eligible: &HashSet<u32>, token: VarId) {
	if let Rvalue::Call(Callee::Builtin(tag, _), args) = rv {
		if tag == "dict-insert"
			&& args
				.get(1)
				.is_some_and(|a| matches!(a, Atom::Var(v) if eligible.contains(&v.0)))
		{
			*tag = "dict-insert-into".into();
			args.push(Atom::Var(token));
		}
	}
}

// --------------------------------------------------------------------------
// Small structural helpers.
// --------------------------------------------------------------------------

/// The set of variables assigned (a `Let` target) anywhere in `b`.
fn assigned_in_block(b: &Block) -> HashSet<u32> {
	let mut out = HashSet::new();
	fn walk(b: &Block, out: &mut HashSet<u32>) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, _) => {
					out.insert(v.0);
				}
				StmtKind::If(_, t, e) => {
					walk(t, out);
					walk(e, out);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, out);
					}
					walk(default, out);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(&arm.body, out);
					}
				}
				StmtKind::Loop(blk) => walk(blk, out),
				_ => {}
			}
		}
	}
	walk(b, &mut out);
	out
}

/// Does this statement *read* `Var(d)` anywhere (excluding a `Let(d, …)` target)?
fn stmt_reads_var(kind: &StmtKind, d: u32) -> bool {
	let mut found = false;
	let mut a = |x: &Atom| {
		if let Atom::Var(v) = x {
			if v.0 == d {
				found = true;
			}
		}
	};
	match kind {
		StmtKind::Let(_, rv) | StmtKind::Discard(rv) => rvalue_atoms(rv, &mut a),
		StmtKind::Return(x) | StmtKind::PushDefer(x) => a(x),
		StmtKind::If(c, _, _) => a(c),
		StmtKind::Switch { scrutinee, .. } => a(scrutinee),
		StmtKind::Match { subject, .. } => a(subject),
		StmtKind::Loop(_) | StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
	}
	found
}

fn rvalue_atoms(rv: &Rvalue, a: &mut impl FnMut(&Atom)) {
	match rv {
		Rvalue::Use(x) | Rvalue::Not(x) | Rvalue::Box(x) | Rvalue::Unbox(x, _) => a(x),
		Rvalue::Bin(_, x, y) => {
			a(x);
			a(y);
		}
		Rvalue::Call(_, args)
		| Rvalue::TailCallDirect(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args)
		| Rvalue::MakeVariant { payload: args, .. } => {
			for x in args {
				a(x);
			}
		}
		Rvalue::CallClosure(g, args) | Rvalue::TailCall(g, args) => {
			a(g);
			for x in args {
				a(x);
			}
		}
		Rvalue::RecordUpdate { base, fields } => {
			a(base);
			for (_, x) in fields {
				a(x);
			}
		}
		Rvalue::MakeRecord(fields) => {
			for (_, x) in fields {
				a(x);
			}
		}
		Rvalue::GetField(x, _, _)
		| Rvalue::GetElement(x, _)
		| Rvalue::GetTag(x)
		| Rvalue::GetPayload(x, _)
		| Rvalue::GetDictMethod(x, _)
		| Rvalue::Await(x) => a(x),
		Rvalue::MakeList(items) => {
			for it in items {
				match it {
					ListItem::Elem(x) | ListItem::Spread(x) => a(x),
				}
			}
		}
		Rvalue::GlobalRef(_) | Rvalue::Builtin(_) | Rvalue::MakeVariantCtor { .. } => {}
	}
}

/// One past the largest `VarId` in the function — the first free id for a new local.
fn next_var(f: &Function) -> u32 {
	let mut max = 0u32;
	let mut bump = |v: u32| max = max.max(v + 1);
	for v in f.params.iter().chain(f.captures.iter()) {
		bump(v.0);
	}
	fn walk(b: &Block, bump: &mut impl FnMut(u32)) {
		for s in &b.0 {
			if let StmtKind::Let(v, _) = &s.kind {
				bump(v.0);
			}
			match &s.kind {
				StmtKind::If(_, t, e) => {
					walk(t, bump);
					walk(e, bump);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, bump);
					}
					walk(default, bump);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						pattern_binds(&arm.pattern, bump);
						walk(&arm.body, bump);
					}
				}
				StmtKind::Loop(blk) => walk(blk, bump),
				_ => {}
			}
		}
	}
	walk(&f.body, &mut bump);
	max
}

fn pattern_binds(p: &Pattern, bump: &mut impl FnMut(u32)) {
	match p {
		Pattern::Bind(v) => bump(v.0),
		Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
			for f in fields {
				pattern_binds(f, bump);
			}
		}
		Pattern::List { items, rest } => {
			for it in items {
				pattern_binds(it, bump);
			}
			if let Some(ListRest::Bind(v)) = rest {
				bump(v.0);
			}
		}
		Pattern::Record { fields, rest, .. } => {
			for (_, f) in fields {
				pattern_binds(f, bump);
			}
			if let RecordRest::Bind(v) = rest {
				bump(v.0);
			}
		}
		Pattern::Wildcard | Pattern::Literal(_) => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use compiler::Range;

	fn syn() -> Range {
		Range::collapsed(0, 0)
	}

	fn st(kind: StmtKind) -> Stmt {
		Stmt::new(kind, syn())
	}

	fn insert(dict: u32) -> Rvalue {
		// dict.insert lowers to a builtin call `[witness, dict, key, val]`.
		Rvalue::Call(
			Callee::Builtin("dict-insert".into(), Repr::Boxed),
			vec![
				Atom::Const(Const::Int(0)),
				Atom::Var(VarId(dict)),
				Atom::Const(Const::Int(0)),
				Atom::Const(Const::Int(0)),
			],
		)
	}

	/// A loopify-shaped `dict.insert` accumulator over params `m=0, i=1`. `extra` is
	/// spliced into the recursive arm before the reassignment (to inject an escape).
	fn accumulator(extra: Vec<Stmt>) -> Function {
		let mut rec = vec![st(StmtKind::Let(VarId(4), insert(0)))];
		rec.extend(extra);
		rec.extend([
			st(StmtKind::Let(VarId(5), Rvalue::Use(Atom::Var(VarId(4))))), // stage m'
			st(StmtKind::Let(
				VarId(6),
				Rvalue::Bin(
					BinOp::SubInt,
					Atom::Var(VarId(1)),
					Atom::Const(Const::Int(1)),
				),
			)),
			st(StmtKind::Let(VarId(7), Rvalue::Use(Atom::Var(VarId(6))))), // stage i'
			st(StmtKind::Let(VarId(0), Rvalue::Use(Atom::Var(VarId(5))))), // m := m'
			st(StmtKind::Let(VarId(1), Rvalue::Use(Atom::Var(VarId(7))))), // i := i'
			st(StmtKind::Continue),
		]);
		let loop_body = Block(vec![
			st(StmtKind::Let(
				VarId(2),
				Rvalue::Bin(
					BinOp::EqI64,
					Atom::Var(VarId(1)),
					Atom::Const(Const::Int(0)),
				),
			)),
			st(StmtKind::Match {
				subject: Atom::Var(VarId(2)),
				arms: vec![
					MatchArm {
						pattern: Pattern::Literal(Const::Bool(true)),
						body: Block(vec![
							st(StmtKind::Let(VarId(3), Rvalue::Use(Atom::Var(VarId(0))))),
							st(StmtKind::Break),
						]),
					},
					MatchArm {
						pattern: Pattern::Wildcard,
						body: Block(rec),
					},
				],
			}),
		]);
		Function {
			name: "acc".into(),
			module: "main".into(),
			params: vec![VarId(0), VarId(1)],
			captures: vec![],
			is_async: false,
			poll_fn: None,
			body: Block(vec![
				st(StmtKind::Loop(loop_body)),
				st(StmtKind::Return(Atom::Var(VarId(3)))),
			]),
			var_reprs: vec![],
			param_reprs: vec![],
			ret_repr: Repr::Boxed,
		}
	}

	fn reuse_one(f: Function) -> Function {
		let mut p = IrProgram {
			functions: vec![f],
			globals: vec![],
			enums: Default::default(),
			entry: FuncId(0),
			test_suites: vec![],
		};
		reuse(&mut p);
		p.functions.pop().unwrap()
	}

	fn count_tag(b: &Block, want: &str) -> usize {
		let mut n = 0;
		for s in &b.0 {
			if let StmtKind::Let(_, Rvalue::Call(Callee::Builtin(tag, _), _)) = &s.kind {
				if tag == want {
					n += 1;
				}
			}
			match &s.kind {
				StmtKind::Loop(blk) => n += count_tag(blk, want),
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						n += count_tag(&arm.body, want);
					}
				}
				StmtKind::If(_, t, e) => n += count_tag(t, want) + count_tag(e, want),
				_ => {}
			}
		}
		n
	}

	#[test]
	fn rewrites_linear_accumulator() {
		let f = reuse_one(accumulator(vec![]));
		// A token is minted before the loop, and the insert became the transient.
		assert!(
			matches!(&f.body.0[0].kind, StmtKind::Let(_, Rvalue::Call(Callee::Builtin(t, _), a)) if t == "dict-mint-token" && a.is_empty()),
			"expected a mint-token prologue, got {:?}",
			f.body.0[0].kind
		);
		assert_eq!(
			count_tag(&f.body, "dict-insert"),
			0,
			"no persistent insert should remain"
		);
		assert_eq!(
			count_tag(&f.body, "dict-insert-into"),
			1,
			"the insert should be transient"
		);
	}

	#[test]
	fn declines_when_result_escapes() {
		// The inserted dict (VarId 4) is also stored in a list — it escapes, so reuse
		// must not fire (the retained copy would be corrupted by the next insert).
		let escape = vec![st(StmtKind::Let(
			VarId(8),
			Rvalue::MakeList(vec![ListItem::Elem(Atom::Var(VarId(4)))]),
		))];
		let f = reuse_one(accumulator(escape));
		assert_eq!(
			count_tag(&f.body, "dict-insert-into"),
			0,
			"escape must block reuse"
		);
		assert_eq!(
			count_tag(&f.body, "dict-insert"),
			1,
			"the persistent insert stays"
		);
	}

	#[test]
	fn declines_when_dict_escapes() {
		// The accumulator `m` (VarId 0) itself is stored — a live alias — so the
		// in-place mutation would be observable. Reuse must decline.
		let escape = vec![st(StmtKind::Let(
			VarId(8),
			Rvalue::MakeList(vec![ListItem::Elem(Atom::Var(VarId(0)))]),
		))];
		let f = reuse_one(accumulator(escape));
		assert_eq!(
			count_tag(&f.body, "dict-insert-into"),
			0,
			"aliasing must block reuse"
		);
	}
}
