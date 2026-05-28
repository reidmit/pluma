// Param/return monomorphization — the WASM-backend perf payoff.
//
// Lowering stamps every function's `param_reprs`/`ret_repr` with the projection
// of its AST types (`repr::repr_of_type`), so a concrete `fib`'s `n` is already
// recorded as `I64`. But giving a function an unboxed calling convention is only
// sound if *every* caller agrees and the function never escapes as a boxed value.
// This pass decides that: it keeps the unboxed signature for **eligible** concrete
// functions and reverts every ineligible one to the uniform-boxed contract
// (all-`Boxed`). Afterwards `repr::Sigs::from_program` reflects the final
// convention, and the interprocedural coercion/validation passes make eligible
// caller↔callee chains pass unboxed values with no `Box`/`Unbox` churn.
//
// **Eligibility** (first cut — non-escaping concrete top-level defs):
//   0. It is a direct-call target — a top-level def whose global holds a bare
//      capture-free closure of it (so all its calls were resolvable). This also
//      excludes async functions (they aren't direct-call targets).
//   1. It has at least one unboxed param or an unboxed return (else nothing to
//      gain — leave it boxed).
//   2. It does not escape: after `resolve_direct_calls`, no `GlobalRef` to its
//      global remains anywhere. A surviving `GlobalRef` means the function is used
//      as a value, or is reached by an unresolved (e.g. tail) call — in either
//      case some site assumes the boxed convention, so it must stay boxed.
//   3. It is closured exactly once (only in its own thunk). A second `MakeClosure`
//      of it is another escape route.
//
// This pass requires `resolve_direct_calls` to have run (it calls it, idempotently)
// — escape analysis is only meaningful once resolvable calls have been rewritten
// and their `GlobalRef` loads pruned.
//
// Like the rest of the Repr track this is inert on the bytecode VM: the unboxed
// signature only changes which `Box`/`Unbox` coercions the repr pass inserts, and
// those are VM no-ops. Its real consumer is the WASM backend.

use crate::types::*;
use std::collections::{HashMap, HashSet};

/// Decide the final calling convention for every function: eligible concrete
/// top-level defs keep their unboxed `param_reprs`/`ret_repr`; everyone else
/// reverts to all-`Boxed`. Idempotent.
pub fn monomorphize(program: &mut IrProgram) {
	crate::resolve::resolve_direct_calls(program);

	let eligible = eligible_functions(program);
	for (fid, f) in program.functions.iter_mut().enumerate() {
		if !eligible.contains(&(fid as u32)) {
			f.param_reprs = vec![Repr::Boxed; f.params.len()];
			f.ret_repr = Repr::Boxed;
		}
	}
}

fn eligible_functions(program: &IrProgram) -> HashSet<u32> {
	// fid -> its global, for top-level defs (invert the direct-call-target map).
	let mut global_of: HashMap<u32, u32> = HashMap::new();
	for (gid, fid) in crate::resolve::direct_call_targets(program) {
		global_of.insert(fid.0, gid);
	}

	// Globals still read as a value anywhere (a surviving `GlobalRef`), and the
	// number of `MakeClosure` sites per function — both escape signals.
	let mut live_globals: HashSet<u32> = HashSet::new();
	let mut closure_sites: HashMap<u32, u32> = HashMap::new();
	for f in &program.functions {
		for_each_rvalue(&f.body, &mut |rv| match rv {
			Rvalue::GlobalRef(g) => {
				live_globals.insert(g.0);
			}
			Rvalue::MakeClosure(fid, _) => {
				*closure_sites.entry(fid.0).or_default() += 1;
			}
			_ => {}
		});
	}

	let mut eligible = HashSet::new();
	for (fid, f) in program.functions.iter().enumerate() {
		let fid = fid as u32;
		// (0) a top-level def (resolvable direct-call target; excludes async).
		let Some(&g) = global_of.get(&fid) else {
			continue;
		};
		// (1) something to gain.
		let has_unboxed = f.param_reprs.iter().any(|r| *r != Repr::Boxed) || f.ret_repr != Repr::Boxed;
		if !has_unboxed {
			continue;
		}
		// (2) does not escape via its global.
		if live_globals.contains(&g) {
			continue;
		}
		// (3) closured only in its own thunk.
		if closure_sites.get(&fid).copied().unwrap_or(0) != 1 {
			continue;
		}
		eligible.insert(fid);
	}
	eligible
}

/// Visit every `Rvalue` in a block (recursing into nested control flow).
fn for_each_rvalue(b: &Block, f: &mut impl FnMut(&Rvalue)) {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => f(rv),
			StmtKind::If(_, t, e) => {
				for_each_rvalue(t, f);
				for_each_rvalue(e, f);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms {
					for_each_rvalue(blk, f);
				}
				for_each_rvalue(default, f);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					for_each_rvalue(&arm.body, f);
				}
			}
			StmtKind::Loop(blk) => for_each_rvalue(blk, f),
			_ => {}
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

	fn func(
		name: &str,
		params: Vec<VarId>,
		param_reprs: Vec<Repr>,
		ret: Repr,
		body: Vec<Stmt>,
	) -> Function {
		Function {
			name: name.into(),
			module: "m".into(),
			params,
			captures: vec![],
			is_async: false,
			body: Block(body),
			var_reprs: vec![],
			param_reprs,
			ret_repr: ret,
		}
	}

	fn thunk_for(fid: u32) -> Function {
		func(
			"thunk",
			vec![],
			vec![],
			Repr::Boxed,
			vec![
				Stmt::new(
					StmtKind::Let(VarId(0), Rvalue::MakeClosure(FuncId(fid), vec![])),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(0))), syn()),
			],
		)
	}

	// A concrete `fib`-shaped def, called only via resolved direct calls, is
	// eligible and keeps its `I64` param / `I64` return.
	#[test]
	fn concrete_non_escaping_def_is_eligible() {
		// fn0 = fib body (I64 param, I64 ret), with a direct self-call; fn1 = thunk.
		let fib = func(
			"fib",
			vec![VarId(0)],
			vec![Repr::I64],
			Repr::I64,
			vec![
				Stmt::new(
					StmtKind::Let(
						VarId(1),
						Rvalue::Call(Callee::Function(FuncId(0)), vec![Atom::Var(VarId(0))]),
					),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()),
			],
		);
		let thunk = thunk_for(0);
		let mut program = IrProgram {
			functions: vec![fib, thunk],
			globals: vec![GlobalInit::Thunk(FuncId(1))],
			enums: Default::default(),
			entry: FuncId(1),
			test_suites: vec![],
			test_new: None,
		};
		monomorphize(&mut program);
		assert_eq!(program.functions[0].param_reprs, vec![Repr::I64]);
		assert_eq!(program.functions[0].ret_repr, Repr::I64);
	}

	// The same def, but its global is still read as a value (it escapes) — it must
	// revert to the uniform-boxed convention.
	#[test]
	fn escaping_def_reverts_to_boxed() {
		let fib = func("fib", vec![VarId(0)], vec![Repr::I64], Repr::I64, vec![]);
		let thunk = thunk_for(0);
		// A function that loads global 0 as a value (escape).
		let user = func(
			"user",
			vec![],
			vec![],
			Repr::Boxed,
			vec![
				Stmt::new(
					StmtKind::Let(VarId(0), Rvalue::GlobalRef(GlobalId(0))),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(0))), syn()),
			],
		);
		let mut program = IrProgram {
			functions: vec![fib, thunk, user],
			globals: vec![GlobalInit::Thunk(FuncId(1))],
			enums: Default::default(),
			entry: FuncId(1),
			test_suites: vec![],
			test_new: None,
		};
		monomorphize(&mut program);
		assert_eq!(program.functions[0].param_reprs, vec![Repr::Boxed]);
		assert_eq!(program.functions[0].ret_repr, Repr::Boxed);
	}

	// A fully-boxed def (no unboxed param or return) has nothing to gain and stays
	// boxed (not "eligible" in any observable way).
	#[test]
	fn all_boxed_def_unchanged() {
		let id = func("id", vec![VarId(0)], vec![Repr::Boxed], Repr::Boxed, vec![]);
		let thunk = thunk_for(0);
		let mut program = IrProgram {
			functions: vec![id, thunk],
			globals: vec![GlobalInit::Thunk(FuncId(1))],
			enums: Default::default(),
			entry: FuncId(1),
			test_suites: vec![],
			test_new: None,
		};
		monomorphize(&mut program);
		assert_eq!(program.functions[0].param_reprs, vec![Repr::Boxed]);
		assert_eq!(program.functions[0].ret_repr, Repr::Boxed);
	}
}
