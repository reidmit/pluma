// Repr (representation) analysis — the step-2 WASM-backend prerequisite.
//
// The bytecode VM is uniformly boxed: its `Value` enum is already inline-tagged,
// so `int`/`float`/`bool` cost nothing extra and there is no boxed/unboxed
// distinction to make. WasmGC is different — it wants `int`→i64, `float`→f64,
// `bool`→i32 *locals*, with explicit coercions wherever an unboxed value meets a
// polymorphic or compound (boxed) context. This module makes that representation
// discipline explicit in the IR so a future WASM emitter can read it off:
//
//   * `infer_reprs`     — assign a `Repr` to every `VarId` (uniform-boxed-first:
//                         only arithmetic/comparison/`Not` results and primitive
//                         `Const` literals are unboxed; everything else is Boxed).
//   * `insert_coercions`— rewrite a function so every operand has the `Repr` its
//                         consumer requires, splicing `Box`/`Unbox` rvalues at the
//                         boundaries.
//   * `validate_reprs`  — the WASM-readiness checker: assert no naked cross-repr
//                         flow remains after coercion.
//
// All of this is inert on the bytecode VM (`Box`/`Unbox` lower to a no-op `Use`,
// and the split comparison ops map back to the VM's polymorphic opcodes). The VM
// anchors are therefore *behavior preservation* (the differential corpus harness)
// and this *static validator* — the real consumer is the WASM backend.
//
// Scope is uniform-boxed-first (per IR.md: "uniform-boxed for generics first;
// monomorphization later"). Function params/captures/returns and every call
// result are `Boxed`; only leaf arithmetic and literals are unboxed. Monomorphizing
// params/returns (so a concrete `fib`'s `n` is an i64 param, eliminating the
// box/unbox churn) and unboxing `Eq`/`GetTag`/`GetPayload` are explicit follow-ons.

use crate::types::*;
use compiler::Range;
use std::collections::{HashMap, HashSet};

// --------------------------------------------------------------------------
// Repr of each binding (inference).
// --------------------------------------------------------------------------

/// Assign a `Repr` to every `VarId` the function defines or uses, indexed by
/// `VarId.0`. Params and captures are `Boxed`; each `Let`'s var takes the natural
/// repr of its rvalue (`result_repr`); pattern-bound vars stay `Boxed` (they bind
/// boxed subjects/payloads). The vector is sized to cover every var the function
/// mentions, so the coercion pass can allocate fresh vars past its end.
///
/// **Join vars are forced `Boxed`.** An `if`/`when` result var is assigned the
/// trailing value of each arm; those values can have different reprs even at one
/// Pluma type (one arm yields `n + 1` = I64, another a boxed call result), so the
/// single var can't take a single unboxed repr. Boxing the join keeps the
/// arithmetic inside each arm unboxed — only the join-point `Use` boxes (the
/// coercer turns the arm's `Let(result, Use(x))` into `Box(x)`).
pub fn infer_reprs(f: &Function) -> Vec<Repr> {
	let joins = find_join_vars(f);
	let mut reprs = vec![Repr::Boxed; var_upper_bound(f)];
	assign_block(&f.body, &mut reprs, &joins);
	reprs
}

fn assign_block(b: &Block, reprs: &mut Vec<Repr>, joins: &HashSet<u32>) {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(v, rv) => {
				reprs[v.0 as usize] = if joins.contains(&v.0) {
					Repr::Boxed
				} else {
					result_repr(rv, reprs)
				};
			}
			StmtKind::If(_, t, e) => {
				assign_block(t, reprs, joins);
				assign_block(e, reprs, joins);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					assign_block(b, reprs, joins);
				}
				assign_block(default, reprs, joins);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					assign_block(&arm.body, reprs, joins);
				}
			}
			StmtKind::Loop(b) => assign_block(b, reprs, joins),
			// These bind no value. Pattern-bound vars (inside `Match` arms) stay
			// at their `Boxed` default.
			StmtKind::Discard(_)
			| StmtKind::Return(_)
			| StmtKind::Break
			| StmtKind::Continue
			| StmtKind::RunDefer(_)
			| StmtKind::PushDefer(_) => {}
		}
	}
}

/// The repr an rvalue's *result* takes, given the reprs already assigned to its
/// operand vars.
pub fn result_repr(rv: &Rvalue, reprs: &[Repr]) -> Repr {
	match rv {
		Rvalue::Bin(op, _, _) => binop_result_repr(*op),
		Rvalue::Not(_) => Repr::I32,
		Rvalue::Use(a) => atom_repr(a, reprs),
		Rvalue::Box(_) => Repr::Boxed,
		Rvalue::Unbox(_, r) => *r,
		// Everything that yields a heap value or a polymorphic value.
		Rvalue::Call(..)
		| Rvalue::CallClosure(..)
		| Rvalue::TailCall(..)
		| Rvalue::GetDictMethod(..)
		| Rvalue::MakeDict(..)
		| Rvalue::MakeClosure(..)
		| Rvalue::MakeRecord(..)
		| Rvalue::GetField(..)
		| Rvalue::MakeVariant { .. }
		| Rvalue::MakeVariantCtor { .. }
		| Rvalue::Interpolate(..)
		| Rvalue::Regex(..)
		| Rvalue::GetTag(..)
		| Rvalue::GetPayload(..)
		| Rvalue::MakeList(..)
		| Rvalue::MakeTuple(..)
		| Rvalue::GlobalRef(..)
		| Rvalue::Builtin(..)
		| Rvalue::Await(..) => Repr::Boxed,
	}
}

fn binop_result_repr(op: BinOp) -> Repr {
	use BinOp::*;
	match op {
		AddInt | SubInt | MulInt | DivInt | RemInt => Repr::I64,
		AddFloat | SubFloat | MulFloat | DivFloat | RemFloat => Repr::F64,
		Concat => Repr::Boxed,
		// All relations and logical ops produce a `bool`.
		And | Or | Eq | Ne | LtI64 | LtF64 | LeI64 | LeF64 | GtI64 | GtF64 | GeI64 | GeF64 => Repr::I32,
	}
}

/// The repr a binary op *requires* of each operand (both operands share it).
fn binop_operand_repr(op: BinOp) -> Repr {
	use BinOp::*;
	match op {
		AddInt | SubInt | MulInt | DivInt | RemInt | LtI64 | LeI64 | GtI64 | GeI64 => Repr::I64,
		AddFloat | SubFloat | MulFloat | DivFloat | RemFloat | LtF64 | LeF64 | GtF64 | GeF64 => {
			Repr::F64
		}
		And | Or => Repr::I32,
		// `==`/`!=` are structural and `++` is string concat: boxed operands.
		Eq | Ne | Concat => Repr::Boxed,
	}
}

fn atom_repr(a: &Atom, reprs: &[Repr]) -> Repr {
	match a {
		Atom::Var(v) => reprs.get(v.0 as usize).copied().unwrap_or(Repr::Boxed),
		Atom::Const(c) => const_repr(c),
	}
}

fn const_repr(c: &Const) -> Repr {
	match c {
		Const::Int(_) => Repr::I64,
		Const::Float(_) => Repr::F64,
		Const::Bool(_) => Repr::I32,
		// `nothing`, strings, bytes, and durations are heap/opaque values.
		Const::Unit | Const::Str(_) | Const::Bytes(_) | Const::Duration(_) => Repr::Boxed,
	}
}

// --------------------------------------------------------------------------
// Operand requirements. A single visitor drives both coercion (mutating each
// operand) and validation (checking each operand on a throwaway clone).
// --------------------------------------------------------------------------

/// Visit each operand atom of an rvalue that carries a representation
/// *requirement*, calling `f(atom, required_repr)`. `Use` is a move (its result
/// repr is its operand's, so no coercion); `Box`/`Unbox` are coercions already;
/// callee positions (`Call`'s `Callee`, `GlobalRef`, …) carry no atom operand.
fn for_each_required_operand(rv: &mut Rvalue, mut f: impl FnMut(&mut Atom, Repr)) {
	match rv {
		Rvalue::Bin(op, a, b) => {
			let r = binop_operand_repr(*op);
			f(a, r);
			f(b, r);
		}
		Rvalue::Not(a) => f(a, Repr::I32),
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			f(c, Repr::Boxed);
			for a in args {
				f(a, Repr::Boxed);
			}
		}
		Rvalue::Call(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => {
			for a in args {
				f(a, Repr::Boxed);
			}
		}
		Rvalue::MakeRecord(fields) => {
			for (_, a) in fields {
				f(a, Repr::Boxed);
			}
		}
		Rvalue::MakeVariant { payload, .. } => {
			for a in payload {
				f(a, Repr::Boxed);
			}
		}
		Rvalue::MakeList(items) => {
			for it in items {
				match it {
					ListItem::Elem(a) | ListItem::Spread(a) => f(a, Repr::Boxed),
				}
			}
		}
		Rvalue::GetDictMethod(a, _)
		| Rvalue::GetField(a, _)
		| Rvalue::GetTag(a)
		| Rvalue::GetPayload(a, _)
		| Rvalue::Await(a) => f(a, Repr::Boxed),
		// No coercible operands: a move, an existing coercion, or operand-free.
		Rvalue::Use(_)
		| Rvalue::Box(_)
		| Rvalue::Unbox(_, _)
		| Rvalue::MakeVariantCtor { .. }
		| Rvalue::Regex(_)
		| Rvalue::GlobalRef(_)
		| Rvalue::Builtin(_) => {}
	}
}

// --------------------------------------------------------------------------
// Coercion insertion (IR -> IR).
// --------------------------------------------------------------------------

/// Rewrite `f` so every operand carries the `Repr` its consumer requires,
/// splicing `Box`/`Unbox` rvalues at each mismatch. After this, `validate_reprs`
/// holds and the function is WASM-ready. Idempotent in effect: re-running inserts
/// nothing (every operand already matches).
pub fn insert_coercions(f: &mut Function) {
	let mut reprs = std::mem::take(&mut f.var_reprs);
	if reprs.is_empty() {
		reprs = infer_reprs(f);
	}
	// Make sure fresh coercion vars start past every existing var.
	let needed = var_upper_bound(f);
	if reprs.len() < needed {
		reprs.resize(needed, Repr::Boxed);
	}
	let mut ctx = Coercer { reprs };
	let body = std::mem::replace(&mut f.body, Block(Vec::new()));
	f.body = ctx.block(body);
	f.var_reprs = ctx.reprs;
}

struct Coercer {
	reprs: Vec<Repr>,
}

impl Coercer {
	fn fresh(&mut self, repr: Repr) -> VarId {
		let v = VarId(self.reprs.len() as u32);
		self.reprs.push(repr);
		v
	}

	/// Coerce `a` to `req`, pushing any needed `Box`/`Unbox` `Let`s into `pre`
	/// and rewriting `a` to the coerced var. All transitions route through
	/// `Boxed` (so the rare unboxed→other-unboxed case is Box-then-Unbox).
	fn coerce(&mut self, a: &mut Atom, req: Repr, pre: &mut Vec<Stmt>, range: Range) {
		let actual = atom_repr(a, &self.reprs);
		if actual == req {
			return;
		}
		let boxed = if actual == Repr::Boxed {
			a.clone()
		} else {
			let v = self.fresh(Repr::Boxed);
			pre.push(Stmt::new(StmtKind::Let(v, Rvalue::Box(a.clone())), range));
			Atom::Var(v)
		};
		*a = if req == Repr::Boxed {
			boxed
		} else {
			let v = self.fresh(req);
			pre.push(Stmt::new(
				StmtKind::Let(v, Rvalue::Unbox(boxed, req)),
				range,
			));
			Atom::Var(v)
		};
	}

	fn block(&mut self, b: Block) -> Block {
		let mut out = Vec::with_capacity(b.0.len());
		for stmt in b.0 {
			self.stmt(stmt, &mut out);
		}
		Block(out)
	}

	fn stmt(&mut self, stmt: Stmt, out: &mut Vec<Stmt>) {
		let range = stmt.range;
		let mut pre = Vec::new();
		let kind = match stmt.kind {
			StmtKind::Let(v, mut rv) => {
				// A `Use` is a move: its result repr is its operand's. Coerce the
				// operand to the *target's* repr so a join `Let(result, Use(x))`
				// (where the result var was forced `Boxed`) boxes `x`, while a plain
				// `let y = x` (target repr == operand repr) inserts nothing.
				if let Rvalue::Use(a) = &mut rv {
					let target = self.reprs[v.0 as usize];
					self.coerce(a, target, &mut pre, range);
				} else {
					for_each_required_operand(&mut rv, |a, r| self.coerce(a, r, &mut pre, range));
				}
				StmtKind::Let(v, rv)
			}
			StmtKind::Discard(mut rv) => {
				for_each_required_operand(&mut rv, |a, r| self.coerce(a, r, &mut pre, range));
				StmtKind::Discard(rv)
			}
			StmtKind::Return(mut a) => {
				self.coerce(&mut a, Repr::Boxed, &mut pre, range);
				StmtKind::Return(a)
			}
			StmtKind::PushDefer(mut a) => {
				self.coerce(&mut a, Repr::Boxed, &mut pre, range);
				StmtKind::PushDefer(a)
			}
			StmtKind::If(mut cond, t, e) => {
				self.coerce(&mut cond, Repr::I32, &mut pre, range);
				StmtKind::If(cond, self.block(t), self.block(e))
			}
			StmtKind::Switch {
				mut scrutinee,
				arms,
				default,
			} => {
				self.coerce(&mut scrutinee, Repr::Boxed, &mut pre, range);
				let arms = arms.into_iter().map(|(t, b)| (t, self.block(b))).collect();
				StmtKind::Switch {
					scrutinee,
					arms,
					default: Box::new(self.block(*default)),
				}
			}
			StmtKind::Match { mut subject, arms } => {
				self.coerce(&mut subject, Repr::Boxed, &mut pre, range);
				let arms = arms
					.into_iter()
					.map(|arm| MatchArm {
						pattern: arm.pattern,
						body: self.block(arm.body),
					})
					.collect();
				StmtKind::Match { subject, arms }
			}
			StmtKind::Loop(b) => StmtKind::Loop(self.block(b)),
			other @ (StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_)) => other,
		};
		out.extend(pre);
		out.push(Stmt::new(kind, range));
	}
}

// --------------------------------------------------------------------------
// Validation: the WASM-readiness checker.
// --------------------------------------------------------------------------

/// Assert the repr discipline holds for `f`: every `Let`'s recorded repr matches
/// its rvalue's result repr, and every operand's repr matches what its consumer
/// requires (so a WASM emitter never sees a boxed value where it needs an i64,
/// nor vice-versa). Run over the whole fixture corpus after `insert_coercions`.
pub fn validate_reprs(f: &Function) -> Result<(), String> {
	check_block(&f.body, &f.var_reprs, &f.name)
}

fn check_block(b: &Block, reprs: &[Repr], fname: &str) -> Result<(), String> {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(v, rv) => {
				let got = reprs.get(v.0 as usize).copied().unwrap_or(Repr::Boxed);
				let want = result_repr(rv, reprs);
				if got != want {
					return Err(format!(
						"{fname}: var {} recorded {got:?} but its rvalue produces {want:?}",
						v.0
					));
				}
				check_rvalue(rv, reprs, fname)?;
			}
			StmtKind::Discard(rv) => check_rvalue(rv, reprs, fname)?,
			StmtKind::Return(a) => require(a, Repr::Boxed, reprs, fname, "return")?,
			StmtKind::PushDefer(a) => require(a, Repr::Boxed, reprs, fname, "defer")?,
			StmtKind::If(cond, t, e) => {
				require(cond, Repr::I32, reprs, fname, "if-cond")?;
				check_block(t, reprs, fname)?;
				check_block(e, reprs, fname)?;
			}
			StmtKind::Switch {
				scrutinee,
				arms,
				default,
			} => {
				require(scrutinee, Repr::Boxed, reprs, fname, "switch")?;
				for (_, b) in arms {
					check_block(b, reprs, fname)?;
				}
				check_block(default, reprs, fname)?;
			}
			StmtKind::Match { subject, arms } => {
				require(subject, Repr::Boxed, reprs, fname, "match")?;
				for arm in arms {
					check_block(&arm.body, reprs, fname)?;
				}
			}
			StmtKind::Loop(b) => check_block(b, reprs, fname)?,
			StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
		}
	}
	Ok(())
}

fn check_rvalue(rv: &Rvalue, reprs: &[Repr], fname: &str) -> Result<(), String> {
	// Reuse the coercion visitor on a throwaway clone: it never mutates here, it
	// just reports the first operand whose repr disagrees with its requirement.
	let mut rv = rv.clone();
	let mut err = None;
	for_each_required_operand(&mut rv, |a, req| {
		let actual = atom_repr(a, reprs);
		if err.is_none() && actual != req {
			err = Some(format!(
				"{fname}: operand {a:?} has repr {actual:?} but its consumer requires {req:?}"
			));
		}
	});
	err.map_or(Ok(()), Err)
}

fn require(a: &Atom, req: Repr, reprs: &[Repr], fname: &str, ctx: &str) -> Result<(), String> {
	let actual = atom_repr(a, reprs);
	if actual == req {
		Ok(())
	} else {
		Err(format!(
			"{fname}: {ctx} operand {a:?} has repr {actual:?} but requires {req:?}"
		))
	}
}

// --------------------------------------------------------------------------
// Var-id upper bound (sizing). Scans every var the function defines or uses.
// --------------------------------------------------------------------------

fn var_upper_bound(f: &Function) -> usize {
	let mut max = 0u32;
	let mut bump = |v: VarId| max = max.max(v.0 + 1);
	for v in f.params.iter().chain(f.captures.iter()) {
		bump(*v);
	}
	block_vars(&f.body, &mut bump);
	max as usize
}

fn block_vars(b: &Block, bump: &mut impl FnMut(VarId)) {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(v, rv) => {
				bump(*v);
				rvalue_vars(rv, bump);
			}
			StmtKind::Discard(rv) => rvalue_vars(rv, bump),
			StmtKind::Return(a) | StmtKind::PushDefer(a) => atom_var(a, bump),
			StmtKind::If(c, t, e) => {
				atom_var(c, bump);
				block_vars(t, bump);
				block_vars(e, bump);
			}
			StmtKind::Switch {
				scrutinee,
				arms,
				default,
			} => {
				atom_var(scrutinee, bump);
				for (_, b) in arms {
					block_vars(b, bump);
				}
				block_vars(default, bump);
			}
			StmtKind::Match { subject, arms } => {
				atom_var(subject, bump);
				for arm in arms {
					pattern_vars(&arm.pattern, bump);
					block_vars(&arm.body, bump);
				}
			}
			StmtKind::Loop(b) => block_vars(b, bump),
			StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
		}
	}
}

fn rvalue_vars(rv: &Rvalue, bump: &mut impl FnMut(VarId)) {
	match rv {
		Rvalue::Use(a) | Rvalue::Not(a) | Rvalue::Box(a) | Rvalue::Unbox(a, _) => atom_var(a, bump),
		Rvalue::Bin(_, a, b) => {
			atom_var(a, bump);
			atom_var(b, bump);
		}
		Rvalue::Call(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => {
			for a in args {
				atom_var(a, bump);
			}
		}
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			atom_var(c, bump);
			for a in args {
				atom_var(a, bump);
			}
		}
		Rvalue::MakeRecord(fields) => {
			for (_, a) in fields {
				atom_var(a, bump);
			}
		}
		Rvalue::MakeVariant { payload, .. } => {
			for a in payload {
				atom_var(a, bump);
			}
		}
		Rvalue::MakeList(items) => {
			for it in items {
				match it {
					ListItem::Elem(a) | ListItem::Spread(a) => atom_var(a, bump),
				}
			}
		}
		Rvalue::GetDictMethod(a, _)
		| Rvalue::GetField(a, _)
		| Rvalue::GetTag(a)
		| Rvalue::GetPayload(a, _)
		| Rvalue::Await(a) => atom_var(a, bump),
		Rvalue::MakeVariantCtor { .. }
		| Rvalue::Regex(_)
		| Rvalue::GlobalRef(_)
		| Rvalue::Builtin(_) => {}
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
		Pattern::Record { fields, rest } => {
			for (_, p) in fields {
				pattern_vars(p, bump);
			}
			if let RecordRest::Bind(v) = rest {
				bump(*v);
			}
		}
		Pattern::Wildcard | Pattern::Literal(_) => {}
	}
}

fn atom_var(a: &Atom, bump: &mut impl FnMut(VarId)) {
	if let Atom::Var(v) = a {
		bump(*v);
	}
}

// --------------------------------------------------------------------------
// Join detection. A var assigned inside conditional branches but read outside
// them is a control-flow join (`if`/`when`/`while` result vars): the runtime
// value reaching its uses comes from whichever arm ran, so its repr can't be
// pinned to one unboxed kind and must be `Boxed`. We find these by giving each
// `Block` an id with a parent link, then flagging any var with a use that no
// assignment block is an ancestor of.
// --------------------------------------------------------------------------

fn find_join_vars(f: &Function) -> HashSet<u32> {
	// Block 0 is the function root; params/captures are "assigned" there.
	let mut parent: Vec<Option<usize>> = vec![None];
	let mut assigns: Vec<(u32, usize)> = Vec::new();
	let mut uses: Vec<(u32, usize)> = Vec::new();
	for v in f.params.iter().chain(f.captures.iter()) {
		assigns.push((v.0, 0));
	}
	walk_join(&f.body, 0, &mut parent, &mut assigns, &mut uses);

	let mut by_var: HashMap<u32, Vec<usize>> = HashMap::new();
	for (v, b) in assigns {
		by_var.entry(v).or_default().push(b);
	}
	let mut joins = HashSet::new();
	for (v, ub) in uses {
		let covered = by_var
			.get(&v)
			.is_some_and(|abs| abs.iter().any(|&ab| is_ancestor(ab, ub, &parent)));
		if !covered {
			joins.insert(v);
		}
	}
	joins
}

fn is_ancestor(anc: usize, mut node: usize, parent: &[Option<usize>]) -> bool {
	loop {
		if node == anc {
			return true;
		}
		match parent[node] {
			Some(p) => node = p,
			None => return false,
		}
	}
}

fn child_block(parent: &mut Vec<Option<usize>>, p: usize) -> usize {
	let id = parent.len();
	parent.push(Some(p));
	id
}

fn walk_join(
	b: &Block,
	block: usize,
	parent: &mut Vec<Option<usize>>,
	assigns: &mut Vec<(u32, usize)>,
	uses: &mut Vec<(u32, usize)>,
) {
	for stmt in &b.0 {
		match &stmt.kind {
			StmtKind::Let(v, rv) => {
				assigns.push((v.0, block));
				rvalue_vars(rv, &mut |u| uses.push((u.0, block)));
			}
			StmtKind::Discard(rv) => rvalue_vars(rv, &mut |u| uses.push((u.0, block))),
			StmtKind::Return(a) | StmtKind::PushDefer(a) => atom_var(a, &mut |u| uses.push((u.0, block))),
			StmtKind::If(c, t, e) => {
				atom_var(c, &mut |u| uses.push((u.0, block)));
				let tb = child_block(parent, block);
				walk_join(t, tb, parent, assigns, uses);
				let eb = child_block(parent, block);
				walk_join(e, eb, parent, assigns, uses);
			}
			StmtKind::Switch {
				scrutinee,
				arms,
				default,
			} => {
				atom_var(scrutinee, &mut |u| uses.push((u.0, block)));
				for (_, blk) in arms {
					let cb = child_block(parent, block);
					walk_join(blk, cb, parent, assigns, uses);
				}
				let db = child_block(parent, block);
				walk_join(default, db, parent, assigns, uses);
			}
			StmtKind::Match { subject, arms } => {
				atom_var(subject, &mut |u| uses.push((u.0, block)));
				for arm in arms {
					let ab = child_block(parent, block);
					pattern_vars(&arm.pattern, &mut |u| assigns.push((u.0, ab)));
					walk_join(&arm.body, ab, parent, assigns, uses);
				}
			}
			StmtKind::Loop(blk) => {
				let cb = child_block(parent, block);
				walk_join(blk, cb, parent, assigns, uses);
			}
			StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn syn() -> Range {
		Range::collapsed(0, 0)
	}

	fn func(params: Vec<VarId>, body: Vec<Stmt>) -> Function {
		Function {
			name: "t".into(),
			module: "m".into(),
			params,
			captures: vec![],
			is_async: false,
			body: Block(body),
			var_reprs: vec![],
		}
	}

	// `fun n { n - 1 }`: n is a Boxed param, the SubInt result is I64.
	#[test]
	fn infers_arithmetic_and_param() {
		let n = VarId(0);
		let t = VarId(1);
		let mut f = func(
			vec![n],
			vec![
				Stmt::new(
					StmtKind::Let(
						t,
						Rvalue::Bin(BinOp::SubInt, Atom::Var(n), Atom::Const(Const::Int(1))),
					),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(t)), syn()),
			],
		);
		f.var_reprs = infer_reprs(&f);
		assert_eq!(f.var_reprs[0], Repr::Boxed); // param n
		assert_eq!(f.var_reprs[1], Repr::I64); // n - 1
	}

	// The same function, coerced: a boxed `n` is unboxed into the SubInt and the
	// I64 result is boxed for the Return. Afterwards it validates.
	#[test]
	fn coerces_unbox_and_box() {
		let n = VarId(0);
		let t = VarId(1);
		let mut f = func(
			vec![n],
			vec![
				Stmt::new(
					StmtKind::Let(
						t,
						Rvalue::Bin(BinOp::SubInt, Atom::Var(n), Atom::Const(Const::Int(1))),
					),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(t)), syn()),
			],
		);
		f.var_reprs = infer_reprs(&f);
		insert_coercions(&mut f);

		// Body is now: Unbox(n)->u ; Let t = u - 1 ; Box(t)->b ; Return b.
		let kinds: Vec<&StmtKind> = f.body.0.iter().map(|s| &s.kind).collect();
		assert!(
			matches!(kinds[0], StmtKind::Let(_, Rvalue::Unbox(_, Repr::I64))),
			"expected leading Unbox, got {:?}",
			kinds[0]
		);
		assert!(matches!(kinds[1], StmtKind::Let(v, Rvalue::Bin(BinOp::SubInt, _, _)) if *v == t));
		assert!(matches!(kinds[2], StmtKind::Let(_, Rvalue::Box(_))));
		assert!(matches!(kinds[3], StmtKind::Return(_)));
		validate_reprs(&f).expect("coerced function must validate");
	}

	// `fun x { x }`: polymorphic identity — everything is Boxed, so no coercions.
	#[test]
	fn polymorphic_identity_needs_no_coercion() {
		let x = VarId(0);
		let mut f = func(
			vec![x],
			vec![Stmt::new(StmtKind::Return(Atom::Var(x)), syn())],
		);
		f.var_reprs = infer_reprs(&f);
		let before = f.body.0.len();
		insert_coercions(&mut f);
		assert_eq!(f.body.0.len(), before, "no coercions expected");
		validate_reprs(&f).expect("identity validates");
	}

	// An int comparison: operands are I64, the result is I32 (bool).
	#[test]
	fn comparison_operand_and_result_reprs() {
		let a = VarId(0);
		let b = VarId(1);
		let r = VarId(2);
		let mut f = func(
			vec![a, b],
			vec![
				Stmt::new(
					StmtKind::Let(r, Rvalue::Bin(BinOp::LtI64, Atom::Var(a), Atom::Var(b))),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(r)), syn()),
			],
		);
		f.var_reprs = infer_reprs(&f);
		assert_eq!(f.var_reprs[2], Repr::I32);
		insert_coercions(&mut f);
		validate_reprs(&f).expect("comparison validates");
		// Both operands (boxed params) get unboxed to I64.
		let unboxes = f
			.body
			.0
			.iter()
			.filter(|s| matches!(s.kind, StmtKind::Let(_, Rvalue::Unbox(_, Repr::I64))))
			.count();
		assert_eq!(unboxes, 2);
	}
}
