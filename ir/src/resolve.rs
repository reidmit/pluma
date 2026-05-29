// Direct-call resolution — the enabling pass for monomorphization.
//
// A call to a top-level function lowers to an *indirect* call through the global
// that holds its closure value: `Let(c, GlobalRef(g)); CallClosure(c, args)`
// (see `lower::lower_call`). That hides the callee's identity at the call site,
// so the coercion pass can't see its signature and must treat every argument and
// result as `Boxed`. This pass recovers the static target: when `g` is a global
// whose initializer is exactly a capture-free closure of a known function, the
// `CallClosure` is rewritten to a direct `Call(Callee::Function(fid), args)`.
//
// That makes the callee visible to the step-2 monomorphization pass (which can
// then give an eligible concrete function unboxed params and require its callers
// to match). It is also a small optimization in its own right — a direct call
// skips the global load and (after dead-ref pruning) the closure indirection.
//
// **Behavior-neutral.** A top-level function captures nothing, so the global
// holds a zero-capture closure of `fid`; `from_ir`'s `Callee::Function` arm emits
// a zero-capture `MakeClosure(fid)` + `Call`, identical to loading that global
// and calling it. The one exception is **async** callees: an awaiting function
// lowers to a cold `Value::AsyncFn` (the global's `MakeClosure` becomes
// `MakeAsyncClosure`), but the `Callee::Function` emit path always builds a plain
// closure — so async targets are deliberately left indirect.
//
// Only `CallClosure` is resolved, not `TailCall`: the IR has no direct-tail-call
// form, and tail-called functions are simply left ineligible for monomorphization
// (a missed optimization, not a correctness gap — the monomorphization pass
// requires *every* use of a function to be a resolved direct call).

use crate::types::*;
use std::collections::{HashMap, HashSet};

/// Rewrite indirect calls to statically-known top-level functions into direct
/// `Call(Callee::Function(..))`s, across the whole program. Idempotent.
pub fn resolve_direct_calls(program: &mut IrProgram) {
	let targets = direct_call_targets(program);
	if targets.is_empty() {
		return;
	}
	for f in &mut program.functions {
		let var_globals = collect_var_globals(f);
		let mut body = std::mem::replace(&mut f.body, Block(Vec::new()));
		rewrite_block(&mut body, &var_globals, &targets);
		f.body = body;
		prune_dead_global_refs(f);
	}
}

/// Map each global index to the function it directly holds, when its initializer
/// is a thunk whose body is exactly `Let(v, MakeClosure(fid, [])); Return(v)` —
/// a capture-free closure returned directly — and `fid` is not async. Public so
/// `mono` identifies monomorphization candidates by the same rule resolution uses
/// (a function is a candidate iff its calls were resolvable to it).
pub fn direct_call_targets(program: &IrProgram) -> HashMap<u32, FuncId> {
	let mut map = HashMap::new();
	for (gid, init) in program.globals.iter().enumerate() {
		let GlobalInit::Thunk(thunk_fid) = init else {
			continue;
		};
		let Some(thunk) = program.functions.get(thunk_fid.0 as usize) else {
			continue;
		};
		if let Some(fid) = closure_returned_by(thunk) {
			let is_async = program
				.functions
				.get(fid.0 as usize)
				.is_some_and(|f| f.is_async);
			if !is_async {
				map.insert(gid as u32, fid);
			}
		}
	}
	map
}

/// If `f`'s body is `Let(v, MakeClosure(fid, [])); Return(Var(v))`, return `fid`.
fn closure_returned_by(f: &Function) -> Option<FuncId> {
	let stmts = &f.body.0;
	if stmts.len() != 2 {
		return None;
	}
	let (bound, fid) = match &stmts[0].kind {
		StmtKind::Let(v, Rvalue::MakeClosure(fid, caps)) if caps.is_empty() => (*v, *fid),
		_ => return None,
	};
	match &stmts[1].kind {
		StmtKind::Return(Atom::Var(rv)) if *rv == bound => Some(fid),
		_ => None,
	}
}

/// Within one function, map each var bound directly to a global to that global's
/// index. `VarId`s are function-unique, so a flat map across nested blocks is
/// unambiguous.
fn collect_var_globals(f: &Function) -> HashMap<u32, u32> {
	let mut map = HashMap::new();
	fn walk(b: &Block, map: &mut HashMap<u32, u32>) {
		for stmt in &b.0 {
			match &stmt.kind {
				StmtKind::Let(v, Rvalue::GlobalRef(g)) => {
					map.insert(v.0, g.0);
				}
				StmtKind::If(_, t, e) => {
					walk(t, map);
					walk(e, map);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						walk(blk, map);
					}
					walk(default, map);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						walk(&arm.body, map);
					}
				}
				StmtKind::Loop(blk) => walk(blk, map),
				_ => {}
			}
		}
	}
	walk(&f.body, &mut map);
	map
}

fn rewrite_block(b: &mut Block, vg: &HashMap<u32, u32>, targets: &HashMap<u32, FuncId>) {
	for stmt in &mut b.0 {
		match &mut stmt.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => rewrite_rvalue(rv, vg, targets),
			StmtKind::If(_, t, e) => {
				rewrite_block(t, vg, targets);
				rewrite_block(e, vg, targets);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					rewrite_block(blk, vg, targets);
				}
				rewrite_block(default, vg, targets);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					rewrite_block(&mut arm.body, vg, targets);
				}
			}
			StmtKind::Loop(blk) => rewrite_block(blk, vg, targets),
			_ => {}
		}
	}
}

fn rewrite_rvalue(rv: &mut Rvalue, vg: &HashMap<u32, u32>, targets: &HashMap<u32, FuncId>) {
	if let Rvalue::CallClosure(Atom::Var(v), args) = rv {
		if let Some(fid) = vg.get(&v.0).and_then(|g| targets.get(g)).copied() {
			let args = std::mem::take(args);
			*rv = Rvalue::Call(Callee::Function(fid), args);
		}
	}
}

/// Drop `Let(v, GlobalRef(_))` bindings whose `v` is no longer used as an
/// operand anywhere — the callee temps the rewrite just orphaned. `GlobalRef` of
/// an already-initialized global is pure, so removing a dead one is safe; this is
/// what turns the rewrite into an actual indirection skip rather than a dead
/// load+store.
fn prune_dead_global_refs(f: &mut Function) {
	let used = used_vars(f);
	fn retain(b: &mut Block, used: &HashSet<u32>) {
		b.0.retain(
			|stmt| !matches!(&stmt.kind, StmtKind::Let(v, Rvalue::GlobalRef(_)) if !used.contains(&v.0)),
		);
		for stmt in &mut b.0 {
			match &mut stmt.kind {
				StmtKind::If(_, t, e) => {
					retain(t, used);
					retain(e, used);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, blk) in arms {
						retain(blk, used);
					}
					retain(default, used);
				}
				StmtKind::Match { arms, .. } => {
					for arm in arms {
						retain(&mut arm.body, used);
					}
				}
				StmtKind::Loop(blk) => retain(blk, used),
				_ => {}
			}
		}
	}
	let mut body = std::mem::replace(&mut f.body, Block(Vec::new()));
	retain(&mut body, &used);
	f.body = body;
}

/// Every var referenced as an operand (not a binding target) anywhere in `f`.
fn used_vars(f: &Function) -> HashSet<u32> {
	let mut used = HashSet::new();
	let mut note = |a: &Atom| {
		if let Atom::Var(v) = a {
			used.insert(v.0);
		}
	};
	fn atoms_of(rv: &Rvalue, note: &mut impl FnMut(&Atom)) {
		match rv {
			Rvalue::Use(a) | Rvalue::Not(a) | Rvalue::Box(a) | Rvalue::Unbox(a, _) => note(a),
			Rvalue::Bin(_, a, b) => {
				note(a);
				note(b);
			}
			Rvalue::Call(_, args)
			| Rvalue::MakeDict(args)
			| Rvalue::MakeTuple(args)
			| Rvalue::Interpolate(args)
			| Rvalue::MakeClosure(_, args) => args.iter().for_each(note),
			Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
				note(c);
				args.iter().for_each(note);
			}
			Rvalue::MakeRecord(fields) => fields.iter().for_each(|(_, a)| note(a)),
			Rvalue::RecordUpdate { base, fields } => {
				note(base);
				fields.iter().for_each(|(_, a)| note(a));
			}
			Rvalue::MakeVariant { payload, .. } => payload.iter().for_each(note),
			Rvalue::MakeList(items) => items.iter().for_each(|it| match it {
				ListItem::Elem(a) | ListItem::Spread(a) => note(a),
			}),
			Rvalue::GetDictMethod(a, _)
			| Rvalue::GetField(a, _)
			| Rvalue::GetElement(a, _)
			| Rvalue::GetTag(a)
			| Rvalue::GetPayload(a, _)
			| Rvalue::Await(a) => note(a),
			Rvalue::MakeVariantCtor { .. }
			| Rvalue::Regex(_)
			| Rvalue::GlobalRef(_)
			| Rvalue::Builtin(_) => {}
		}
	}
	fn walk(b: &Block, note: &mut impl FnMut(&Atom)) {
		for stmt in &b.0 {
			match &stmt.kind {
				StmtKind::Let(_, rv) | StmtKind::Discard(rv) => atoms_of(rv, note),
				StmtKind::Return(a) | StmtKind::PushDefer(a) => note(a),
				StmtKind::If(c, t, e) => {
					note(c);
					walk(t, note);
					walk(e, note);
				}
				StmtKind::Switch {
					scrutinee,
					arms,
					default,
				} => {
					note(scrutinee);
					for (_, blk) in arms {
						walk(blk, note);
					}
					walk(default, note);
				}
				StmtKind::Match { subject, arms } => {
					note(subject);
					for arm in arms {
						walk(&arm.body, note);
					}
				}
				StmtKind::Loop(blk) => walk(blk, note),
				StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
			}
		}
	}
	walk(&f.body, &mut note);
	used
}

#[cfg(test)]
mod tests {
	use super::*;
	use compiler::Range;

	fn syn() -> Range {
		Range::collapsed(0, 0)
	}

	fn boxed_fn(name: &str, params: Vec<VarId>, body: Vec<Stmt>) -> Function {
		Function {
			name: name.into(),
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

	/// A thunk whose body is `Let(0, MakeClosure(fid, [])); Return(0)`.
	fn thunk_for(fid: u32) -> Function {
		boxed_fn(
			"thunk",
			vec![],
			vec![
				Stmt::new(
					StmtKind::Let(VarId(0), Rvalue::MakeClosure(FuncId(fid), vec![])),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(0))), syn()),
			],
		)
	}

	/// A caller body: `Let(0, GlobalRef(g)); Let(1, CallClosure(0, [arg])); Return(1)`.
	fn caller_calling(global: u32, arg: Atom) -> Vec<Stmt> {
		vec![
			Stmt::new(
				StmtKind::Let(VarId(0), Rvalue::GlobalRef(GlobalId(global))),
				syn(),
			),
			Stmt::new(
				StmtKind::Let(
					VarId(1),
					Rvalue::CallClosure(Atom::Var(VarId(0)), vec![arg]),
				),
				syn(),
			),
			Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()),
		]
	}

	// A call through a global holding a capture-free closure resolves to a direct
	// call, and the orphaned GlobalRef load is pruned.
	#[test]
	fn resolves_call_through_global() {
		// fn0 = the callee body; fn1 = its thunk; fn2 = a caller of global 0.
		let callee = boxed_fn("callee", vec![VarId(0)], vec![]);
		let thunk = thunk_for(0);
		let caller = boxed_fn(
			"caller",
			vec![],
			caller_calling(0, Atom::Const(Const::Int(1))),
		);
		let mut program = IrProgram {
			functions: vec![callee, thunk, caller],
			globals: vec![GlobalInit::Thunk(FuncId(1))],
			enums: Default::default(),
			entry: FuncId(2),
			test_suites: vec![],
			test_new: None,
		};
		resolve_direct_calls(&mut program);

		let body = &program.functions[2].body.0;
		// The GlobalRef load is gone; the call is now a direct Call to fn0.
		assert_eq!(body.len(), 2, "dead GlobalRef should be pruned");
		assert!(
			matches!(
				&body[0].kind,
				StmtKind::Let(_, Rvalue::Call(Callee::Function(FuncId(0)), _))
			),
			"expected direct Call(Callee::Function(0)), got {:?}",
			body[0].kind
		);
	}

	// Self-recursion: the callee's own body calls itself through its global.
	#[test]
	fn resolves_self_recursion() {
		let recur = boxed_fn(
			"fib",
			vec![VarId(2)],
			caller_calling(0, Atom::Var(VarId(2))),
		);
		let thunk = thunk_for(0);
		let mut program = IrProgram {
			functions: vec![recur, thunk],
			globals: vec![GlobalInit::Thunk(FuncId(1))],
			enums: Default::default(),
			entry: FuncId(1),
			test_suites: vec![],
			test_new: None,
		};
		resolve_direct_calls(&mut program);
		let body = &program.functions[0].body.0;
		assert!(body.iter().any(|s| matches!(
			&s.kind,
			StmtKind::Let(_, Rvalue::Call(Callee::Function(FuncId(0)), _))
		)));
	}

	// An async target is left indirect (its global holds a cold AsyncFn, which the
	// direct-call emit path doesn't reproduce).
	#[test]
	fn leaves_async_target_indirect() {
		let mut callee = boxed_fn("awaits", vec![VarId(0)], vec![]);
		callee.is_async = true;
		let thunk = thunk_for(0);
		let caller = boxed_fn(
			"caller",
			vec![],
			caller_calling(0, Atom::Const(Const::Unit)),
		);
		let mut program = IrProgram {
			functions: vec![callee, thunk, caller],
			globals: vec![GlobalInit::Thunk(FuncId(1))],
			enums: Default::default(),
			entry: FuncId(2),
			test_suites: vec![],
			test_new: None,
		};
		resolve_direct_calls(&mut program);
		let body = &program.functions[2].body.0;
		assert!(
			body
				.iter()
				.any(|s| matches!(&s.kind, StmtKind::Let(_, Rvalue::CallClosure(..)))),
			"async target must stay an indirect CallClosure"
		);
	}

	// A global whose thunk is not a bare capture-free closure (e.g. a computed
	// value, or a builtin) is not a direct-call target.
	#[test]
	fn leaves_non_closure_global_indirect() {
		// thunk returns a global ref, not a MakeClosure -> not resolvable.
		let weird_thunk = boxed_fn(
			"thunk",
			vec![],
			vec![
				Stmt::new(
					StmtKind::Let(VarId(0), Rvalue::GlobalRef(GlobalId(0))),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(0))), syn()),
			],
		);
		let caller = boxed_fn(
			"caller",
			vec![],
			caller_calling(0, Atom::Const(Const::Unit)),
		);
		let mut program = IrProgram {
			functions: vec![weird_thunk, caller],
			globals: vec![GlobalInit::Thunk(FuncId(0))],
			enums: Default::default(),
			entry: FuncId(1),
			test_suites: vec![],
			test_new: None,
		};
		resolve_direct_calls(&mut program);
		let body = &program.functions[1].body.0;
		assert!(
			body
				.iter()
				.any(|s| matches!(&s.kind, StmtKind::Let(_, Rvalue::CallClosure(..)))),
			"non-closure global must stay an indirect CallClosure"
		);
	}
}
