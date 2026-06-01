// Redundant-copy elimination (M2a).
//
// The inliner binds every spliced call's return with `Let(dest, Use(ret))`
// (`inline::splice`), and codegen lowers a `Let(_, Use(Var ..))` to a `Move`. In
// call-heavy code those copies dominate — roughly one per inlined call, ~20% of
// executed opcodes in `helper-calls`. This pass removes them.
//
// A `Let(x, Use(Var y))` whose target `x` is assigned *exactly once* makes `x` a
// pure alias of `y`: substitute `x -> y` at every use and drop the `Let`. ANF's
// single-assignment makes this sound — `y`'s value can't change before `x`'s uses,
// and `y` (defined earlier) dominates them. Multi-def vars — the `if`/`when`/`loop`
// join results, written `Let(result, Use(arm))` once per arm — are deliberately
// left alone: one `result` def isn't enough to alias it to a single arm. Copy
// *chains* (`a = b; c = a`) resolve transitively to the root. Const copies
// (`Let(x, Use(Const ..))`) are left as-is — they lower to a load, not a move, and
// propagating a const into multiple uses can *add* loads.
//
// Behaviour-neutral; runs after `inline`/`resolve` in `ir::optimize`. Anchored by
// the `ir_mono`/`ir_repr` behaviour-neutrality harnesses and cross-backend
// conformance (the VM oracle runs this; the deploy backends don't).

use crate::types::*;
use std::collections::HashMap;

/// Eliminate single-def `Let(x, Use(Var y))` copies across every function.
pub fn eliminate_copies(program: &mut IrProgram) {
	for f in &mut program.functions {
		eliminate_in_function(f);
	}
}

fn eliminate_in_function(f: &mut Function) {
	// 1. How many times is each VarId *defined* (a `Let` target or a pattern bind)?
	//    A copy whose target has a single def is a safe alias.
	let mut def_count: HashMap<u32, u32> = HashMap::new();
	count_defs(&f.body, &mut def_count);

	// 2. Collect single-def `Let(x, Use(Var y))` copies as `x -> y`.
	let mut subst: HashMap<u32, VarId> = HashMap::new();
	collect_copies(&f.body, &def_count, &mut subst);
	if subst.is_empty() {
		return;
	}

	// 3. Resolve chains (`a -> b -> c` becomes `a -> c`). No cycles: single
	//    assignment means a copy's source is defined before its target.
	let roots: HashMap<u32, VarId> = subst
		.keys()
		.map(|&x| (x, resolve(VarId(x), &subst)))
		.collect();

	// 4. Substitute uses and drop the eliminated copy statements.
	rewrite_block(&mut f.body, &roots);
}

fn resolve(v: VarId, subst: &HashMap<u32, VarId>) -> VarId {
	let mut cur = v;
	while let Some(&next) = subst.get(&cur.0) {
		cur = next;
	}
	cur
}

// --------------------------------------------------------------------------
// Pass 1: count definitions.
// --------------------------------------------------------------------------

fn count_defs(b: &Block, counts: &mut HashMap<u32, u32>) {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(v, _) => *counts.entry(v.0).or_default() += 1,
			StmtKind::If(_, t, e) => {
				count_defs(t, counts);
				count_defs(e, counts);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					count_defs(blk, counts);
				}
				count_defs(default, counts);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					count_pattern_binds(&arm.pattern, counts);
					count_defs(&arm.body, counts);
				}
			}
			StmtKind::Loop(blk) => count_defs(blk, counts),
			_ => {}
		}
	}
}

fn count_pattern_binds(p: &Pattern, counts: &mut HashMap<u32, u32>) {
	match p {
		Pattern::Bind(v) => *counts.entry(v.0).or_default() += 1,
		Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
			for f in fields {
				count_pattern_binds(f, counts);
			}
		}
		Pattern::List { items, rest } => {
			for it in items {
				count_pattern_binds(it, counts);
			}
			if let Some(ListRest::Bind(v)) = rest {
				*counts.entry(v.0).or_default() += 1;
			}
		}
		Pattern::Record { fields, rest, .. } => {
			for (_, p) in fields {
				count_pattern_binds(p, counts);
			}
			if let RecordRest::Bind(v) = rest {
				*counts.entry(v.0).or_default() += 1;
			}
		}
		Pattern::Wildcard | Pattern::Literal(_) => {}
	}
}

// --------------------------------------------------------------------------
// Pass 2: collect eliminable copies.
// --------------------------------------------------------------------------

fn collect_copies(b: &Block, def_count: &HashMap<u32, u32>, subst: &mut HashMap<u32, VarId>) {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(x, Rvalue::Use(Atom::Var(y))) => {
				if def_count.get(&x.0).copied().unwrap_or(0) == 1 {
					subst.insert(x.0, *y);
				}
			}
			StmtKind::If(_, t, e) => {
				collect_copies(t, def_count, subst);
				collect_copies(e, def_count, subst);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					collect_copies(blk, def_count, subst);
				}
				collect_copies(default, def_count, subst);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					collect_copies(&arm.body, def_count, subst);
				}
			}
			StmtKind::Loop(blk) => collect_copies(blk, def_count, subst),
			_ => {}
		}
	}
}

// --------------------------------------------------------------------------
// Pass 3: rewrite uses + drop eliminated copies.
// --------------------------------------------------------------------------

fn rewrite_block(b: &mut Block, roots: &HashMap<u32, VarId>) {
	b.0.retain_mut(|stmt| {
		// Drop the eliminated copy itself (its target is the unique def, so any
		// `Let(x, _)` with `x` in `roots` is exactly that copy).
		if let StmtKind::Let(x, _) = &stmt.kind {
			if roots.contains_key(&x.0) {
				return false;
			}
		}
		rewrite_stmt(stmt, roots);
		true
	});
}

fn rewrite_stmt(stmt: &mut Stmt, roots: &HashMap<u32, VarId>) {
	match &mut stmt.kind {
		StmtKind::Let(_, rv) | StmtKind::Discard(rv) => rewrite_rvalue(rv, roots),
		StmtKind::Return(a) | StmtKind::PushDefer(a) => rewrite_atom(a, roots),
		StmtKind::If(cond, t, e) => {
			rewrite_atom(cond, roots);
			rewrite_block(t, roots);
			rewrite_block(e, roots);
		}
		StmtKind::Switch {
			scrutinee,
			arms,
			default,
		} => {
			rewrite_atom(scrutinee, roots);
			for (_, blk) in arms {
				rewrite_block(blk, roots);
			}
			rewrite_block(default, roots);
		}
		StmtKind::Match { subject, arms } => {
			rewrite_atom(subject, roots);
			for arm in arms {
				rewrite_block(&mut arm.body, roots);
			}
		}
		StmtKind::Loop(blk) => rewrite_block(blk, roots),
		StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
	}
}

fn rewrite_rvalue(rv: &mut Rvalue, roots: &HashMap<u32, VarId>) {
	match rv {
		Rvalue::Use(a)
		| Rvalue::Not(a)
		| Rvalue::Box(a)
		| Rvalue::Unbox(a, _)
		| Rvalue::GetDictMethod(a, _)
		| Rvalue::GetField(a, _, _)
		| Rvalue::GetElement(a, _)
		| Rvalue::GetTag(a)
		| Rvalue::GetPayload(a, _)
		| Rvalue::Await(a) => rewrite_atom(a, roots),
		Rvalue::Bin(_, a, b) => {
			rewrite_atom(a, roots);
			rewrite_atom(b, roots);
		}
		Rvalue::Call(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => {
			for a in args {
				rewrite_atom(a, roots);
			}
		}
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			rewrite_atom(c, roots);
			for a in args {
				rewrite_atom(a, roots);
			}
		}
		Rvalue::MakeRecord(fields) => {
			for (_, a) in fields {
				rewrite_atom(a, roots);
			}
		}
		Rvalue::RecordUpdate { base, fields } => {
			rewrite_atom(base, roots);
			for (_, a) in fields {
				rewrite_atom(a, roots);
			}
		}
		Rvalue::MakeVariant { payload, .. } => {
			for a in payload {
				rewrite_atom(a, roots);
			}
		}
		Rvalue::MakeList(items) => {
			for it in items {
				match it {
					ListItem::Elem(a) | ListItem::Spread(a) => rewrite_atom(a, roots),
				}
			}
		}
		Rvalue::MakeVariantCtor { .. } | Rvalue::GlobalRef(_) | Rvalue::Builtin(_) => {}
	}
}

fn rewrite_atom(a: &mut Atom, roots: &HashMap<u32, VarId>) {
	if let Atom::Var(v) = a {
		if let Some(&root) = roots.get(&v.0) {
			*v = root;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use compiler::Range;

	fn syn() -> Range {
		Range::collapsed(0, 0)
	}

	fn func(body: Vec<Stmt>) -> Function {
		Function {
			name: "t".into(),
			module: "m".into(),
			params: vec![VarId(0)],
			captures: vec![],
			is_async: false,
			poll_fn: None,
			body: Block(body),
			var_reprs: vec![],
			param_reprs: vec![],
			ret_repr: Repr::Boxed,
		}
	}

	// `Let(t, n+1); Let(a, Use(t)); Return(a)` — the single-def copy `a = t` is
	// removed and the `Return` rewritten to use `t` directly.
	#[test]
	fn eliminates_single_def_copy() {
		let n = VarId(0);
		let t = VarId(1);
		let a = VarId(2);
		let mut f = func(vec![
			Stmt::new(
				StmtKind::Let(
					t,
					Rvalue::Bin(BinOp::AddInt, Atom::Var(n), Atom::Const(Const::Int(1))),
				),
				syn(),
			),
			Stmt::new(StmtKind::Let(a, Rvalue::Use(Atom::Var(t))), syn()),
			Stmt::new(StmtKind::Return(Atom::Var(a)), syn()),
		]);
		eliminate_in_function(&mut f);
		assert_eq!(
			f.body.0.len(),
			2,
			"the copy `Let(a, Use(t))` should be dropped"
		);
		assert!(matches!(
			&f.body.0[0].kind,
			StmtKind::Let(v, Rvalue::Bin(BinOp::AddInt, _, _)) if *v == t
		));
		assert!(
			matches!(&f.body.0[1].kind, StmtKind::Return(Atom::Var(v)) if *v == t),
			"the return should now read `t`, got {:?}",
			f.body.0[1].kind
		);
	}

	// Copy chains resolve to the root: `Let(a, Use(t)); Let(b, Use(a)); Return(b)`
	// -> both copies dropped, `Return` reads `t`.
	#[test]
	fn resolves_copy_chains() {
		let t = VarId(1);
		let a = VarId(2);
		let b = VarId(3);
		let mut f = func(vec![
			Stmt::new(
				StmtKind::Let(
					t,
					Rvalue::Bin(
						BinOp::AddInt,
						Atom::Var(VarId(0)),
						Atom::Const(Const::Int(1)),
					),
				),
				syn(),
			),
			Stmt::new(StmtKind::Let(a, Rvalue::Use(Atom::Var(t))), syn()),
			Stmt::new(StmtKind::Let(b, Rvalue::Use(Atom::Var(a))), syn()),
			Stmt::new(StmtKind::Return(Atom::Var(b)), syn()),
		]);
		eliminate_in_function(&mut f);
		assert_eq!(f.body.0.len(), 2);
		assert!(matches!(&f.body.0[1].kind, StmtKind::Return(Atom::Var(v)) if *v == t));
	}

	// A join var (two defs, one per `if` arm) is NOT eliminated — substituting it
	// to one arm's value would be wrong.
	#[test]
	fn leaves_join_vars_alone() {
		let result = VarId(3);
		// if cond { result = a } else { result = b } ; return result
		let mut f = func(vec![
			Stmt::new(
				StmtKind::If(
					Atom::Var(VarId(0)),
					Block(vec![Stmt::new(
						StmtKind::Let(result, Rvalue::Use(Atom::Var(VarId(1)))),
						syn(),
					)]),
					Block(vec![Stmt::new(
						StmtKind::Let(result, Rvalue::Use(Atom::Var(VarId(2)))),
						syn(),
					)]),
				),
				syn(),
			),
			Stmt::new(StmtKind::Return(Atom::Var(result)), syn()),
		]);
		let before = format!("{:?}", f.body);
		eliminate_in_function(&mut f);
		assert_eq!(
			format!("{:?}", f.body),
			before,
			"a multi-def join var must be left untouched"
		);
	}
}
