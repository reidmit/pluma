// Loopify — turn self-tail-recursion into a structured `Loop`.
//
// Self-tail-recursion is *not* loopified by lowering: a tail call to the current
// function lowers to `Let(v, TailCallDirect(self, args)); Return(v)` — an
// inter-procedural call that reuses the frame, but still a call (the function
// re-enters from the top each iteration). This pass rewrites such a function so
// the recursion becomes one `StmtKind::Loop` over its parameters: each recursive
// arm reassigns the parameter locals from the call's arguments and `Continue`s;
// each base arm binds the returned value and `Break`s; the loop's value is
// returned afterward.
//
// Why: it is independently a small win (no per-iteration call), but the real
// motivation is downstream reuse analysis (see `notes/REUSE.md`). After
// loopification a tail-recursive accumulator like `tally m … = … tally (insert m …)`
// becomes an *intra-function* loop carrying `m`, where the uniqueness analysis can
// prove `m` is single-threaded and thread a transient token through the inserts.
// Inter-procedural threading would otherwise need a call-graph fixpoint.
//
// **Behavior-neutral.** A `TailCallDirect(self, args)` re-enters the function with
// `params := args`; a `Loop`+`Continue` with `params := args` is the same state
// transition, and the base arms return the same values. The pass only fires on a
// shape it can prove equivalent (every control path in the dispatching match ends
// in a recognized base `Return` or self-tail-call, the match is exhaustive, and no
// self-tail-call hides in an unrecognized position) and otherwise leaves the
// function untouched.
//
// Runs in `wasm::emit`'s pipeline after `resolve_direct_calls` (so the self-call is
// a `TailCallDirect` and `self` is identified by the function's own index) and
// before `infer_reprs` (so the new param-reassignment and result locals get reprs;
// a reassigned param becomes a join var and stays `Boxed`, which the repr/coercion
// passes already handle).

use crate::types::*;
use compiler::Range;

/// Loopify every function whose body is a recognizable self-tail-recursive shape.
pub fn loopify(program: &mut IrProgram) {
	for (idx, f) in program.functions.iter_mut().enumerate() {
		loopify_fn(FuncId(idx as u32), f);
	}
}

fn loopify_fn(self_id: FuncId, f: &mut Function) {
	let n = f.body.0.len();
	if n == 0 {
		return;
	}
	// The body is `head…, Match{…}` optionally followed by the dead `Return(Unit)`
	// epilogue that tail-lowering appends. Locate the dispatching match.
	let dispatch = if n >= 2
		&& matches!(f.body.0[n - 1].kind, StmtKind::Return(_))
		&& matches!(f.body.0[n - 2].kind, StmtKind::Match { .. })
	{
		n - 2
	} else if matches!(f.body.0[n - 1].kind, StmtKind::Match { .. }) {
		n - 1
	} else {
		return;
	};

	let StmtKind::Match { arms, .. } = &f.body.0[dispatch].kind else {
		return;
	};
	let arity = f.params.len();

	// Dry-run check (no mutation): every arm transformable, the match exhaustive
	// (final arm a catch-all), at least one self-tail-call, and *every* self-tail-call
	// in the body sits in a recognized recursive terminator.
	if !matches!(arms.last().map(|a| &a.pattern), Some(Pattern::Wildcard)) {
		return;
	}
	let mut recognized = 0usize;
	for arm in arms {
		match check_block(self_id, &arm.body, arity) {
			Some(rec) => recognized += rec,
			None => return,
		}
	}
	if recognized == 0 || recognized != count_self_tailcalls(self_id, &f.body) {
		return;
	}

	// Commit. Fresh locals live above every existing `VarId`.
	let mut next = next_var(f);
	let result = VarId(next);
	next += 1;

	// Transform each arm's terminators, then assemble `Loop { head…, Match }` and
	// return the loop's result. `head` (which computes the match subject) is moved
	// inside the loop so it recomputes against the reassigned params each iteration.
	let params = f.params.clone();
	let mut body = std::mem::replace(&mut f.body.0, Vec::new());
	body.truncate(dispatch + 1); // drop the dead `Return(Unit)` epilogue, if any
	let dispatch_stmt = body.pop().unwrap();
	let head = body; // stmts before the match
	let range = dispatch_stmt.range;
	let StmtKind::Match { subject, arms } = dispatch_stmt.kind else {
		unreachable!("checked above");
	};
	let arms = arms
		.into_iter()
		.map(|arm| MatchArm {
			pattern: arm.pattern,
			body: transform_block(self_id, arm.body, &params, result, &mut next),
		})
		.collect();

	let mut loop_body = head;
	loop_body.push(Stmt::new(StmtKind::Match { subject, arms }, range));
	f.body.0 = vec![
		Stmt::new(StmtKind::Loop(Block(loop_body)), range),
		Stmt::new(StmtKind::Return(Atom::Var(result)), range),
	];
}

/// Is `block` a transformable terminator tree? Returns the number of recognized
/// recursive (self-tail-call) terminators it contains, or `None` if any control
/// path ends in something other than a base `Return` or a self-tail-call.
fn check_block(self_id: FuncId, block: &Block, arity: usize) -> Option<usize> {
	let stmts = &block.0;
	let n = stmts.len();
	if n == 0 {
		return None;
	}
	// Recursive terminator: `Let(v, TailCallDirect(self, args)); Return(v)`.
	if n >= 2 {
		if let (
			StmtKind::Let(v, Rvalue::TailCallDirect(callee, args)),
			StmtKind::Return(Atom::Var(rv)),
		) = (&stmts[n - 2].kind, &stmts[n - 1].kind)
		{
			if *callee == self_id && *rv == *v && args.len() == arity {
				return Some(1);
			}
		}
	}
	match &stmts[n - 1].kind {
		// Base terminator.
		StmtKind::Return(_) => Some(0),
		// Nested dispatch: recurse. `If` is always two-way (exhaustive); a nested
		// `Match` must be a catch-all match like the top-level one.
		StmtKind::If(_, t, e) => {
			Some(check_block(self_id, t, arity)? + check_block(self_id, e, arity)?)
		}
		StmtKind::Match { arms, .. } => {
			if !matches!(arms.last().map(|a| &a.pattern), Some(Pattern::Wildcard)) {
				return None;
			}
			let mut rec = 0;
			for arm in arms {
				rec += check_block(self_id, &arm.body, arity)?;
			}
			Some(rec)
		}
		_ => None,
	}
}

/// Rewrite a transformable block's terminators: base `Return(a)` becomes
/// `Let(result, Use(a)); Break`; the recursive `Let(v, TailCallDirect); Return(v)`
/// becomes a parallel reassignment of the params followed by `Continue`.
fn transform_block(
	self_id: FuncId,
	block: Block,
	params: &[VarId],
	result: VarId,
	next: &mut u32,
) -> Block {
	let mut stmts = block.0;
	let n = stmts.len();

	// Recursive terminator (matches `check_block`).
	if n >= 2 {
		let is_rec = matches!(
			(&stmts[n - 2].kind, &stmts[n - 1].kind),
			(StmtKind::Let(_, Rvalue::TailCallDirect(c, _)), StmtKind::Return(_)) if *c == self_id
		);
		if is_rec {
			let ret = stmts.pop().unwrap();
			let call = stmts.pop().unwrap();
			let StmtKind::Let(_, Rvalue::TailCallDirect(_, args)) = call.kind else {
				unreachable!()
			};
			let range = ret.range;
			emit_reassign(&mut stmts, params, args, next, range);
			stmts.push(Stmt::new(StmtKind::Continue, range));
			return Block(stmts);
		}
	}

	let last = stmts.pop().expect("non-empty per check_block");
	match last.kind {
		StmtKind::Return(atom) => {
			let range = last.range;
			stmts.push(Stmt::new(StmtKind::Let(result, Rvalue::Use(atom)), range));
			stmts.push(Stmt::new(StmtKind::Break, range));
		}
		StmtKind::If(cond, t, e) => {
			let t = transform_block(self_id, t, params, result, next);
			let e = transform_block(self_id, e, params, result, next);
			stmts.push(Stmt::new(StmtKind::If(cond, t, e), last.range));
		}
		StmtKind::Match { subject, arms } => {
			let arms = arms
				.into_iter()
				.map(|arm| MatchArm {
					pattern: arm.pattern,
					body: transform_block(self_id, arm.body, params, result, next),
				})
				.collect();
			stmts.push(Stmt::new(StmtKind::Match { subject, arms }, last.range));
		}
		_ => unreachable!("transform_block on a block check_block rejected"),
	}
	Block(stmts)
}

/// Emit `params := args` as a parallel assignment: stage each non-identity arg
/// into a fresh temp, then write the temps into the params. Staging avoids
/// clobbering when an argument reads a param that an earlier assignment overwrites
/// (e.g. `f(a, b) = f(b, a)`). Identity args (`p := p`) are skipped.
fn emit_reassign(
	stmts: &mut Vec<Stmt>,
	params: &[VarId],
	args: Vec<Atom>,
	next: &mut u32,
	range: Range,
) {
	let mut temps: Vec<Option<VarId>> = Vec::with_capacity(args.len());
	for (p, a) in params.iter().zip(&args) {
		if matches!(a, Atom::Var(v) if v == p) {
			temps.push(None); // identity — nothing to do
			continue;
		}
		let t = VarId(*next);
		*next += 1;
		stmts.push(Stmt::new(StmtKind::Let(t, Rvalue::Use(a.clone())), range));
		temps.push(Some(t));
	}
	for (p, t) in params.iter().zip(&temps) {
		if let Some(t) = t {
			stmts.push(Stmt::new(
				StmtKind::Let(*p, Rvalue::Use(Atom::Var(*t))),
				range,
			));
		}
	}
}

/// Count `TailCallDirect(self, …)` occurrences anywhere in the body, so we can
/// confirm every one sits in a recognized recursive terminator before committing.
fn count_self_tailcalls(self_id: FuncId, body: &Block) -> usize {
	fn walk(self_id: FuncId, b: &Block, acc: &mut usize) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(_, Rvalue::TailCallDirect(c, _)) if *c == self_id => *acc += 1,
				StmtKind::If(_, t, e) => {
					walk(self_id, t, acc);
					walk(self_id, e, acc);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(self_id, blk, acc);
					}
					walk(self_id, default, acc);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(self_id, &arm.body, acc);
					}
				}
				StmtKind::Loop(blk) => walk(self_id, blk, acc),
				_ => {}
			}
		}
	}
	let mut acc = 0;
	walk(self_id, body, &mut acc);
	acc
}

/// One past the largest `VarId` used anywhere in the function — the first id free
/// for new locals.
fn next_var(f: &Function) -> u32 {
	let mut max = 0u32;
	let mut bump = |v: VarId| max = max.max(v.0 + 1);
	for v in f.params.iter().chain(f.captures.iter()) {
		bump(*v);
	}
	fn walk(b: &Block, bump: &mut impl FnMut(VarId)) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, rv) => {
					bump(*v);
					rvalue_vars(rv, bump);
				}
				StmtKind::Discard(rv) => rvalue_vars(rv, bump),
				StmtKind::Return(a) | StmtKind::PushDefer(a) => atom_var(a, bump),
				StmtKind::If(c, t, e) => {
					atom_var(c, bump);
					walk(t, bump);
					walk(e, bump);
				}
				StmtKind::Switch {
					scrutinee,
					arms,
					default,
				} => {
					atom_var(scrutinee, bump);
					for (_, blk) in arms {
						walk(blk, bump);
					}
					walk(default, bump);
				}
				StmtKind::Match { subject, arms } => {
					atom_var(subject, bump);
					for arm in arms {
						pattern_vars(&arm.pattern, bump);
						walk(&arm.body, bump);
					}
				}
				StmtKind::Loop(blk) => walk(blk, bump),
				StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
			}
		}
	}
	walk(&f.body, &mut bump);
	max
}

fn atom_var(a: &Atom, bump: &mut impl FnMut(VarId)) {
	if let Atom::Var(v) = a {
		bump(*v);
	}
}

fn rvalue_vars(rv: &Rvalue, bump: &mut impl FnMut(VarId)) {
	let mut a = |x: &Atom| atom_var(x, bump);
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
		Rvalue::CallClosure(f, args) | Rvalue::TailCall(f, args) => {
			a(f);
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

fn pattern_vars(p: &Pattern, bump: &mut impl FnMut(VarId)) {
	match p {
		Pattern::Bind(v) => bump(*v),
		Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
			for f in fields {
				pattern_vars(f, bump);
			}
		}
		Pattern::List { items, rest } => {
			for it in items {
				pattern_vars(it, bump);
			}
			if let Some(ListRest::Bind(v)) = rest {
				bump(*v);
			}
		}
		Pattern::Record { fields, rest, .. } => {
			for (_, f) in fields {
				pattern_vars(f, bump);
			}
			if let RecordRest::Bind(v) = rest {
				bump(*v);
			}
		}
		Pattern::Wildcard | Pattern::Literal(_) => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn syn() -> Range {
		Range::collapsed(0, 0)
	}

	fn st(kind: StmtKind) -> Stmt {
		Stmt::new(kind, syn())
	}

	fn func(params: Vec<VarId>, body: Vec<Stmt>) -> Function {
		Function {
			name: "t".into(),
			module: "m".into(),
			params,
			captures: vec![],
			is_async: false,
			poll_fn: None,
			body: Block(body),
			var_reprs: vec![],
			param_reprs: vec![],
			ret_repr: Repr::Boxed,
		}
	}

	// `fn(n, acc) = if n is true { acc } else { self(n-1, acc) }` — the canonical
	// accumulator shape. Becomes `Loop { Match { true => result=acc, Break; _ =>
	// reassign params, Continue } } ; Return(result)`.
	fn accumulator() -> Function {
		let body = vec![
			st(StmtKind::Match {
				subject: Atom::Var(VarId(0)),
				arms: vec![
					MatchArm {
						pattern: Pattern::Literal(Const::Bool(true)),
						body: Block(vec![st(StmtKind::Return(Atom::Var(VarId(1))))]),
					},
					MatchArm {
						pattern: Pattern::Wildcard,
						body: Block(vec![
							st(StmtKind::Let(
								VarId(2),
								Rvalue::Bin(
									BinOp::SubInt,
									Atom::Var(VarId(0)),
									Atom::Const(Const::Int(1)),
								),
							)),
							st(StmtKind::Let(
								VarId(3),
								Rvalue::TailCallDirect(FuncId(0), vec![Atom::Var(VarId(2)), Atom::Var(VarId(1))]),
							)),
							st(StmtKind::Return(Atom::Var(VarId(3)))),
						]),
					},
				],
			}),
			st(StmtKind::Return(Atom::Const(Const::Unit))),
		];
		func(vec![VarId(0), VarId(1)], body)
	}

	fn loopify_one(f: Function) -> Function {
		let mut p = IrProgram {
			functions: vec![f],
			globals: vec![],
			enums: Default::default(),
			entry: FuncId(0),
			test_suites: vec![],
		};
		loopify(&mut p);
		p.functions.pop().unwrap()
	}

	#[test]
	fn loopifies_accumulator() {
		let f = loopify_one(accumulator());
		// Body is now `Loop { Match { ... } } ; Return(result)`.
		assert_eq!(f.body.0.len(), 2);
		let StmtKind::Loop(body) = &f.body.0[0].kind else {
			panic!("expected Loop, got {:?}", f.body.0[0].kind);
		};
		let StmtKind::Return(Atom::Var(result)) = &f.body.0[1].kind else {
			panic!("expected Return(result), got {:?}", f.body.0[1].kind);
		};
		// The loop body holds the Match; no TailCallDirect survives.
		assert_eq!(count_self_tailcalls(FuncId(0), body), 0);
		let StmtKind::Match { arms, .. } = &body.0[0].kind else {
			panic!("expected Match in loop body");
		};
		// Base arm: ends `Let(result, Use(acc)); Break`.
		let base = &arms[0].body.0;
		assert!(matches!(base.last().unwrap().kind, StmtKind::Break));
		assert!(
			matches!(&base[base.len() - 2].kind, StmtKind::Let(v, Rvalue::Use(Atom::Var(VarId(1)))) if v == result),
			"base arm should bind result := acc, got {:?}",
			base
		);
		// Recursive arm: ends in Continue, and reassigns the params.
		let rec = &arms[1].body.0;
		assert!(matches!(rec.last().unwrap().kind, StmtKind::Continue));
		assert!(
			rec
				.iter()
				.any(|s| matches!(&s.kind, StmtKind::Let(VarId(0), _))),
			"recursive arm should reassign param 0, got {:?}",
			rec
		);
	}

	#[test]
	fn leaves_non_recursive_function_untouched() {
		// `fn(n) = n` — no self-tail-call; must be left as-is.
		let f = func(
			vec![VarId(0)],
			vec![st(StmtKind::Return(Atom::Var(VarId(0))))],
		);
		let out = loopify_one(f);
		assert!(matches!(out.body.0[0].kind, StmtKind::Return(_)));
		assert!(!matches!(out.body.0[0].kind, StmtKind::Loop(_)));
	}

	#[test]
	fn leaves_non_self_tailcall_untouched() {
		// A tail call to a *different* function (FuncId(7)) is not loopifiable.
		let body = vec![
			st(StmtKind::Match {
				subject: Atom::Var(VarId(0)),
				arms: vec![
					MatchArm {
						pattern: Pattern::Literal(Const::Bool(true)),
						body: Block(vec![st(StmtKind::Return(Atom::Var(VarId(0))))]),
					},
					MatchArm {
						pattern: Pattern::Wildcard,
						body: Block(vec![
							st(StmtKind::Let(
								VarId(1),
								Rvalue::TailCallDirect(FuncId(7), vec![Atom::Var(VarId(0))]),
							)),
							st(StmtKind::Return(Atom::Var(VarId(1)))),
						]),
					},
				],
			}),
			st(StmtKind::Return(Atom::Const(Const::Unit))),
		];
		let out = loopify_one(func(vec![VarId(0)], body));
		assert!(
			matches!(out.body.0[0].kind, StmtKind::Match { .. }),
			"should stay a Match"
		);
	}

	#[test]
	fn bails_when_match_not_exhaustive() {
		// Final arm is not a catch-all: a fall-through path would loop forever, so
		// the pass must decline.
		let body = vec![
			st(StmtKind::Match {
				subject: Atom::Var(VarId(0)),
				arms: vec![
					MatchArm {
						pattern: Pattern::Literal(Const::Bool(true)),
						body: Block(vec![st(StmtKind::Return(Atom::Var(VarId(0))))]),
					},
					MatchArm {
						pattern: Pattern::Literal(Const::Bool(false)),
						body: Block(vec![
							st(StmtKind::Let(
								VarId(1),
								Rvalue::TailCallDirect(FuncId(0), vec![Atom::Var(VarId(0))]),
							)),
							st(StmtKind::Return(Atom::Var(VarId(1)))),
						]),
					},
				],
			}),
			st(StmtKind::Return(Atom::Const(Const::Unit))),
		];
		let out = loopify_one(func(vec![VarId(0)], body));
		assert!(
			matches!(out.body.0[0].kind, StmtKind::Match { .. }),
			"non-exhaustive match should bail"
		);
	}
}
