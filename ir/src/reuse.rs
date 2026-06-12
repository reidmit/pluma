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
		report_list_fn(f, &mut notes);
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

/// Rewrite proven-safe value-semantics accumulators to in-place mutation: the
/// `dict.insert` transient, and the `[...acc, v]` list spread-append (which would
/// otherwise rebuild the whole backing array each iteration — O(n²) over the scan).
pub fn reuse(program: &mut IrProgram) {
	for f in &mut program.functions {
		reuse_fn(f);
		list_reuse_fn(f);
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

// ==========================================================================
// List append reuse — `[...acc, v]` in a loop-carried accumulator.
//
// A spread literal copies: `acc' = [...acc, v]` builds a fresh backing array of
// length+1 and copies every element, so threading it through a loop is O(n²) over
// the accumulation (the JSON/regex `find-all` footgun). `list.push` appends in
// place (amortized O(1)) by mutating the `$list` struct's array+length fields — but
// only an author who knows the list is uniquely owned can reach for it.
//
// This pass proves that ownership statically for the same loopify accumulator shape
// dict reuse handles, then rewrites `[...acc, e1..en]` (a *leading* spread of the
// accumulator, trailing plain elements) to in-place `list.push`es. Two things make
// it sound:
//   * Every use of `acc` is the leading spread of such an append or the return path
//     — no alias can observe the in-place growth — and `acc` is only ever reassigned
//     from its own appends (never a foreign, possibly-shared list).
//   * One `[...acc]` clone before the loop takes ownership of the initial value, so
//     even if the caller still holds the list it passed in, the pushes mutate our
//     private copy. The clone is O(initial length) — free for the empty-list start
//     an accumulator almost always has.
// Prepend (`[v, ...acc]`) can't append in place (it would shift every element), so
// it stays a copy: `classify_makelist` reports it as an escape and the pass declines.
// "When unsure, copy" — getting it wrong is silent heap corruption.
// --------------------------------------------------------------------------

fn list_reuse_fn(f: &mut Function) {
	// The loopify shape — a `Loop` with a trailing `Return(Var(result))`. Dict reuse
	// may have prepended a token `Let`, so locate the loop rather than demand it at [0].
	let Some((loop_idx, result)) = loop_shape(&f.body) else {
		return;
	};
	let StmtKind::Loop(loop_body) = &f.body.0[loop_idx].kind else {
		unreachable!()
	};

	let reassigned = assigned_in_block(loop_body);
	let params = f.params.clone();
	let mut eligible: Vec<u32> = Vec::new();
	for p in &params {
		if !reassigned.contains(&p.0) {
			continue;
		}
		if list_eligible(f, p.0, result, loop_body) {
			eligible.push(p.0);
		}
	}
	if eligible.is_empty() {
		return;
	}

	let range = f.body.0[loop_idx].range;
	let elig_set: HashSet<u32> = eligible.iter().copied().collect();
	let StmtKind::Loop(loop_body) = &mut f.body.0[loop_idx].kind else {
		unreachable!()
	};
	rewrite_list_block(loop_body, &elig_set);
	// Clone each accumulator once before the loop so the pushes mutate a freshly-owned
	// array. `[...acc]` is a copy; for the usual empty start it costs nothing.
	for &acc in &eligible {
		f.body.0.insert(
			loop_idx,
			Stmt::new(
				StmtKind::Let(
					VarId(acc),
					Rvalue::MakeList(vec![ListItem::Spread(Atom::Var(VarId(acc)))]),
				),
				range,
			),
		);
	}
}

/// Analyze (without rewriting) every `[...acc, …]` append site and report whether the
/// pass would push in place or copy — the list counterpart to `report_fn`, feeding the
/// same soundness harness.
fn report_list_fn(f: &Function, notes: &mut Vec<ReuseNote>) {
	let Some((loop_idx, result)) = loop_shape(&f.body) else {
		return;
	};
	let StmtKind::Loop(loop_body) = &f.body.0[loop_idx].kind else {
		unreachable!()
	};
	let reassigned = assigned_in_block(loop_body);
	for acc in append_sites(&f.body) {
		let reused = reassigned.contains(&acc)
			&& f.params.iter().any(|p| p.0 == acc)
			&& list_eligible(f, acc, result, loop_body);
		notes.push(ReuseNote {
			module: f.module.clone(),
			reused,
		});
	}
}

/// The accumulator var of every `[...acc, …]` leading-spread append in `b`.
fn append_sites(b: &Block) -> Vec<u32> {
	let mut out = Vec::new();
	fn walk(b: &Block, out: &mut Vec<u32>) {
		for s in &b.0 {
			if let StmtKind::Let(_, Rvalue::MakeList(items)) = &s.kind {
				if let Some(ListItem::Spread(Atom::Var(x))) = items.first() {
					if matches!(classify_makelist(items, x.0), MlUse::Append) {
						out.push(x.0);
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

/// Locate the loopify shape: the index of the (single) `Loop` and the `result` var of
/// a trailing `Return(Var(result))`. Tolerates `Let` prologues before the loop.
fn loop_shape(body: &Block) -> Option<(usize, u32)> {
	let n = body.0.len();
	if n < 2 {
		return None;
	}
	let StmtKind::Return(Atom::Var(r)) = &body.0[n - 1].kind else {
		return None;
	};
	let loop_idx = body
		.0
		.iter()
		.position(|s| matches!(s.kind, StmtKind::Loop(_)))?;
	if loop_idx >= n - 1 {
		return None;
	}
	Some((loop_idx, r.0))
}

/// How a `MakeList` mentions the accumulator `acc`.
enum MlUse {
	/// `[...acc, e1..en]` — a leading spread of `acc`, then plain elements that don't
	/// reference `acc` (possibly zero: a bare `[...acc]`). The rewritable append.
	Append,
	/// `acc` appears, but not as a clean leading append — a prepend (`[v, ...acc]`),
	/// a non-leading spread, `acc` as an element, etc. Must stay a copy.
	Escape,
	/// `acc` does not appear.
	Absent,
}

fn classify_makelist(items: &[ListItem], acc: u32) -> MlUse {
	let reads_acc = |a: &Atom| matches!(a, Atom::Var(v) if v.0 == acc);
	let appears = items.iter().any(|it| match it {
		ListItem::Elem(a) | ListItem::Spread(a) => reads_acc(a),
	});
	if !appears {
		return MlUse::Absent;
	}
	if let Some(ListItem::Spread(a)) = items.first() {
		if reads_acc(a)
			&& items[1..]
				.iter()
				.all(|it| matches!(it, ListItem::Elem(e) if !reads_acc(e)))
		{
			return MlUse::Append;
		}
	}
	MlUse::Escape
}

fn is_append_consume(kind: &StmtKind, acc: u32) -> bool {
	matches!(
		kind,
		StmtKind::Let(_, Rvalue::MakeList(items))
			if matches!(classify_makelist(items, acc), MlUse::Append)
	)
}

fn list_eligible(f: &Function, acc: u32, result: u32, loop_body: &Block) -> bool {
	// (a) No escape: every read of `acc` is a leading-spread append or the return copy.
	let mut scan = ListUseScan {
		acc,
		result,
		ok: true,
		consumes: 0,
	};
	scan.block(&f.body);
	if !scan.ok || scan.consumes == 0 {
		return false;
	}
	// (b) Dead-after: each append is the last read of `acc` before it's reassigned.
	if !list_dead_after_ok(loop_body, acc) {
		return false;
	}
	// (c) Each append's result flows only into `acc`'s reassignment.
	let reads = read_counts(f);
	let results = collect_append_results(loop_body, acc);
	if !results
		.iter()
		.all(|&r| result_flows_to_d(f, r, acc, &reads))
	{
		return false;
	}
	// (d) `acc` is only ever reassigned from its own appends — never a foreign list.
	// This is what lets the single upfront clone keep `acc` uniquely owned throughout.
	acc_only_from_appends(f, acc, &results, &reads)
}

/// Classify every read of `acc` across the function: a leading-spread append consume,
/// the return-path copy, or an escape (anything that may retain or alias it).
struct ListUseScan {
	acc: u32,
	result: u32,
	ok: bool,
	consumes: usize,
}

impl ListUseScan {
	fn is_acc(&self, a: &Atom) -> bool {
		matches!(a, Atom::Var(v) if v.0 == self.acc)
	}

	fn escape_if_acc(&mut self, a: &Atom) {
		if self.is_acc(a) {
			self.ok = false;
		}
	}

	fn block(&mut self, b: &Block) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, rv) => self.rvalue(rv, v.0 == self.result),
				StmtKind::Discard(rv) => self.rvalue(rv, false),
				// Returning the final accumulator hands an owned list to the caller; the
				// pass stops mutating it, so this is not an escape (same as dict reuse).
				StmtKind::Return(_) => {}
				StmtKind::PushDefer(a) => self.escape_if_acc(a),
				StmtKind::If(c, t, e) => {
					self.escape_if_acc(c);
					self.block(t);
					self.block(e);
				}
				StmtKind::Switch {
					scrutinee,
					arms,
					default,
				} => {
					self.escape_if_acc(scrutinee);
					for (_, blk) in arms {
						self.block(blk);
					}
					self.block(default);
				}
				StmtKind::Match { subject, arms } => {
					self.escape_if_acc(subject);
					for arm in arms {
						self.block(&arm.body);
					}
				}
				StmtKind::Loop(blk) => self.block(blk),
				StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
			}
		}
	}

	/// `is_result_target` is true for the RHS of `Let(result, …)` — the one place a
	/// bare `Use(Var(acc))` (loopify's copy of a returned accumulator) is allowed.
	fn rvalue(&mut self, rv: &Rvalue, is_result_target: bool) {
		match rv {
			Rvalue::Use(a) => {
				if self.is_acc(a) && !is_result_target {
					self.ok = false;
				}
			}
			Rvalue::MakeList(items) => match classify_makelist(items, self.acc) {
				MlUse::Append => self.consumes += 1,
				MlUse::Escape => self.ok = false,
				MlUse::Absent => {}
			},
			// Every other rvalue: any mention of `acc` may retain or alias it — reject.
			other => {
				let acc = self.acc;
				let mut escapes = false;
				rvalue_atoms(other, &mut |a| {
					if matches!(a, Atom::Var(v) if v.0 == acc) {
						escapes = true;
					}
				});
				if escapes {
					self.ok = false;
				}
			}
		}
	}
}

/// The `r` of every `Let(r, [...acc, …])` append in `b` (recursively).
fn collect_append_results(b: &Block, acc: u32) -> Vec<u32> {
	let mut out = Vec::new();
	fn walk(b: &Block, acc: u32, out: &mut Vec<u32>) {
		for s in &b.0 {
			if let StmtKind::Let(r, _) = &s.kind {
				if is_append_consume(&s.kind, acc) {
					out.push(r.0);
				}
			}
			match &s.kind {
				StmtKind::If(_, t, e) => {
					walk(t, acc, out);
					walk(e, acc, out);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, acc, out);
					}
					walk(default, acc, out);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(&arm.body, acc, out);
					}
				}
				StmtKind::Loop(blk) => walk(blk, acc, out),
				_ => {}
			}
		}
	}
	walk(b, acc, &mut out);
	out
}

/// Each `[...acc, …]` append must be the last read of `acc` (in its block) before
/// `acc` is reassigned — so the in-place push isn't observed by a later read of the
/// old value. Mirrors dict reuse's `dead_after_ok` for the append consume.
fn list_dead_after_ok(b: &Block, acc: u32) -> bool {
	for (i, s) in b.0.iter().enumerate() {
		if is_append_consume(&s.kind, acc) {
			let mut reassigned = false;
			for later in &b.0[i + 1..] {
				if let StmtKind::Let(v, _) = &later.kind {
					if v.0 == acc {
						reassigned = true;
						break;
					}
				}
				if stmt_reads_var(&later.kind, acc) {
					return false;
				}
			}
			if !reassigned {
				return false;
			}
		}
		match &s.kind {
			StmtKind::If(_, t, e) => {
				if !list_dead_after_ok(t, acc) || !list_dead_after_ok(e, acc) {
					return false;
				}
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					if !list_dead_after_ok(blk, acc) {
						return false;
					}
				}
				if !list_dead_after_ok(default, acc) {
					return false;
				}
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					if !list_dead_after_ok(&arm.body, acc) {
						return false;
					}
				}
			}
			StmtKind::Loop(blk) => {
				if !list_dead_after_ok(blk, acc) {
					return false;
				}
			}
			_ => {}
		}
	}
	true
}

/// Every reassignment `Let(acc, rhs)` must take `rhs` from `acc`'s own append results
/// (through loopify's single-use copy staging) — never a foreign list that could be
/// aliased. Without this, the upfront clone wouldn't guarantee continued ownership.
fn acc_only_from_appends(
	f: &Function,
	acc: u32,
	results: &[u32],
	reads: &std::collections::HashMap<u32, usize>,
) -> bool {
	// Forward closure of single-use `Let(x, Use(Var(s)))` copies from the append results.
	let mut owned: HashSet<u32> = results.iter().copied().collect();
	let copies = all_copies(&f.body);
	loop {
		let mut added = false;
		for &(x, s) in &copies {
			if owned.contains(&s) && reads.get(&s).copied().unwrap_or(0) == 1 && owned.insert(x) {
				added = true;
			}
		}
		if !added {
			break;
		}
	}
	// Every write of `acc` must be a `Use` of an owned (append-derived) var.
	let mut ok = true;
	fn walk(b: &Block, acc: u32, owned: &HashSet<u32>, ok: &mut bool) {
		for s in &b.0 {
			if let StmtKind::Let(v, rhs) = &s.kind {
				if v.0 == acc && !matches!(rhs, Rvalue::Use(Atom::Var(src)) if owned.contains(&src.0)) {
					*ok = false;
				}
			}
			match &s.kind {
				StmtKind::If(_, t, e) => {
					walk(t, acc, owned, ok);
					walk(e, acc, owned, ok);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, acc, owned, ok);
					}
					walk(default, acc, owned, ok);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(&arm.body, acc, owned, ok);
					}
				}
				StmtKind::Loop(blk) => walk(blk, acc, owned, ok),
				_ => {}
			}
		}
	}
	walk(&f.body, acc, &owned, &mut ok);
	ok
}

/// Every `Let(x, Use(Var(s)))` copy in the function, as `(x, s)`.
fn all_copies(b: &Block) -> Vec<(u32, u32)> {
	let mut out = Vec::new();
	fn walk(b: &Block, out: &mut Vec<(u32, u32)>) {
		for s in &b.0 {
			if let StmtKind::Let(x, Rvalue::Use(Atom::Var(src))) = &s.kind {
				out.push((x.0, src.0));
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

/// If `items` is `[...acc, e1..en]` with `acc` eligible, return `(acc, [e1..en])`.
fn append_consume_of_eligible(
	items: &[ListItem],
	eligible: &HashSet<u32>,
) -> Option<(u32, Vec<Atom>)> {
	let ListItem::Spread(Atom::Var(acc)) = items.first()? else {
		return None;
	};
	if !eligible.contains(&acc.0) {
		return None;
	}
	let mut elems = Vec::with_capacity(items.len() - 1);
	for it in &items[1..] {
		match it {
			ListItem::Elem(a) => elems.push(a.clone()),
			ListItem::Spread(_) => return None,
		}
	}
	Some((acc.0, elems))
}

/// Rewrite each `Let(r, [...acc, e1..en])` of an eligible `acc` to in-place pushes
/// (`list.push acc ei`) followed by `Let(r, Use(acc))` — `r` is the same growing list.
fn rewrite_list_block(b: &mut Block, eligible: &HashSet<u32>) {
	let mut out = Vec::with_capacity(b.0.len());
	for mut s in b.0.drain(..) {
		match &mut s.kind {
			StmtKind::If(_, t, e) => {
				rewrite_list_block(t, eligible);
				rewrite_list_block(e, eligible);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					rewrite_list_block(blk, eligible);
				}
				rewrite_list_block(default, eligible);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					rewrite_list_block(&mut arm.body, eligible);
				}
			}
			StmtKind::Loop(blk) => rewrite_list_block(blk, eligible),
			_ => {}
		}
		let rng = s.range;
		if let StmtKind::Let(r, Rvalue::MakeList(items)) = &s.kind {
			if let Some((acc, elems)) = append_consume_of_eligible(items, eligible) {
				let r = *r;
				for e in elems {
					out.push(Stmt::new(
						StmtKind::Discard(Rvalue::Call(
							Callee::Builtin("list-push".into(), Repr::Boxed),
							vec![Atom::Var(VarId(acc)), e],
						)),
						rng,
					));
				}
				out.push(Stmt::new(
					StmtKind::Let(r, Rvalue::Use(Atom::Var(VarId(acc)))),
					rng,
				));
				continue;
			}
		}
		out.push(s);
	}
	b.0 = out;
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

	// ---- list append reuse ----

	/// A loopify-shaped list accumulator over params `acc=0, i=1`. The consume is
	/// `Let(4, MakeList(consume_items))`; `extra` is spliced after it (to inject an
	/// escape). Mirrors `accumulator` but threads a list, not a dict.
	fn list_accumulator(consume_items: Vec<ListItem>, extra: Vec<Stmt>) -> Function {
		let mut rec = vec![st(StmtKind::Let(VarId(4), Rvalue::MakeList(consume_items)))];
		rec.extend(extra);
		rec.extend([
			st(StmtKind::Let(VarId(5), Rvalue::Use(Atom::Var(VarId(4))))), // stage acc'
			st(StmtKind::Let(
				VarId(6),
				Rvalue::Bin(
					BinOp::SubInt,
					Atom::Var(VarId(1)),
					Atom::Const(Const::Int(1)),
				),
			)),
			st(StmtKind::Let(VarId(7), Rvalue::Use(Atom::Var(VarId(6))))), // stage i'
			st(StmtKind::Let(VarId(0), Rvalue::Use(Atom::Var(VarId(5))))), // acc := acc'
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

	/// `[...acc, v]` — the leading-spread append the pass rewrites.
	fn append() -> Vec<ListItem> {
		vec![
			ListItem::Spread(Atom::Var(VarId(0))),
			ListItem::Elem(Atom::Const(Const::Int(7))),
		]
	}

	/// True when `f`'s body opens with a `[...acc]` clone prologue for VarId(0).
	fn has_clone_prologue(f: &Function) -> bool {
		f.body.0.iter().any(|s| {
			matches!(&s.kind, StmtKind::Let(v, Rvalue::MakeList(items))
				if v.0 == 0
					&& matches!(items.as_slice(), [ListItem::Spread(Atom::Var(a))] if a.0 == 0))
		})
	}

	/// Count builtin calls tagged `want` anywhere in `b`, whether bound (`Let`) or
	/// discarded for effect (`Discard`) — the in-place `list.push` is a `Discard`.
	fn count_builtin(b: &Block, want: &str) -> usize {
		let mut n = 0;
		let hit = |rv: &Rvalue| matches!(rv, Rvalue::Call(Callee::Builtin(tag, _), _) if tag == want);
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(_, rv) | StmtKind::Discard(rv) if hit(rv) => n += 1,
				_ => {}
			}
			match &s.kind {
				StmtKind::Loop(blk) => n += count_builtin(blk, want),
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						n += count_builtin(&arm.body, want);
					}
				}
				StmtKind::If(_, t, e) => n += count_builtin(t, want) + count_builtin(e, want),
				_ => {}
			}
		}
		n
	}

	/// Count `MakeList`s anywhere in `b` whose first item is `Spread` (appends/clones).
	fn count_spread_lists(b: &Block) -> usize {
		let mut n = 0;
		for s in &b.0 {
			if let StmtKind::Let(_, Rvalue::MakeList(items)) = &s.kind {
				if matches!(items.first(), Some(ListItem::Spread(_))) {
					n += 1;
				}
			}
			match &s.kind {
				StmtKind::Loop(blk) => n += count_spread_lists(blk),
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						n += count_spread_lists(&arm.body);
					}
				}
				StmtKind::If(_, t, e) => n += count_spread_lists(t) + count_spread_lists(e),
				_ => {}
			}
		}
		n
	}

	#[test]
	fn rewrites_list_append_accumulator() {
		let f = reuse_one(list_accumulator(append(), vec![]));
		// One `[...acc]` clone before the loop takes ownership.
		assert!(has_clone_prologue(&f), "expected a clone prologue");
		// The append inside the loop became an in-place push of the single element.
		assert_eq!(
			count_builtin(&f.body, "list-push"),
			1,
			"the appended element should become a list.push"
		);
		// The only spread-`MakeList` left is the clone prologue — the in-loop append is gone.
		assert_eq!(
			count_spread_lists(&f.body),
			1,
			"the in-loop spread-append must be replaced by pushes"
		);
	}

	#[test]
	fn rewrites_multi_element_append() {
		// `[...acc, a, b]` → two pushes, then `acc` flows on.
		let items = vec![
			ListItem::Spread(Atom::Var(VarId(0))),
			ListItem::Elem(Atom::Const(Const::Int(7))),
			ListItem::Elem(Atom::Const(Const::Int(9))),
		];
		let f = reuse_one(list_accumulator(items, vec![]));
		assert!(has_clone_prologue(&f));
		assert_eq!(
			count_builtin(&f.body, "list-push"),
			2,
			"one push per element"
		);
	}

	#[test]
	fn declines_on_prepend() {
		// `[v, ...acc]` can't append in place (it shifts every element) — stay a copy.
		let prepend = vec![
			ListItem::Elem(Atom::Const(Const::Int(7))),
			ListItem::Spread(Atom::Var(VarId(0))),
		];
		let f = reuse_one(list_accumulator(prepend, vec![]));
		assert_eq!(
			count_builtin(&f.body, "list-push"),
			0,
			"prepend is not rewritten"
		);
		assert!(
			!has_clone_prologue(&f),
			"no clone when nothing is rewritten"
		);
	}

	#[test]
	fn declines_when_list_escapes() {
		// The accumulator (VarId 0) is also stored in another list — a live alias — so
		// the in-place push would be observable. Reuse must decline.
		let escape = vec![st(StmtKind::Let(
			VarId(8),
			Rvalue::MakeList(vec![ListItem::Elem(Atom::Var(VarId(0)))]),
		))];
		let f = reuse_one(list_accumulator(append(), escape));
		assert_eq!(
			count_builtin(&f.body, "list-push"),
			0,
			"aliasing must block reuse"
		);
		assert!(!has_clone_prologue(&f));
	}

	#[test]
	fn declines_when_append_result_escapes() {
		// The appended list (VarId 4) is stored elsewhere — the retained copy would be
		// corrupted by a later in-place push. Reuse must decline.
		let escape = vec![st(StmtKind::Let(
			VarId(8),
			Rvalue::MakeList(vec![ListItem::Elem(Atom::Var(VarId(4)))]),
		))];
		let f = reuse_one(list_accumulator(append(), escape));
		assert_eq!(
			count_builtin(&f.body, "list-push"),
			0,
			"escape must block reuse"
		);
	}
}
