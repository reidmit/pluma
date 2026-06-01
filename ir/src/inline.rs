// Function inlining — splice small, non-recursive, directly-called functions
// into their call sites.
//
// Pluma is accessor- and combinator-heavy: a `def name = fun p { p.field }`
// accessor or a `def inc = fun x { x + 1 }` one-liner is, at every use, a full
// call — frame push, argument copy, `Return`, stack truncate. The IR is ANF with
// function-unique `VarId`s, so inlining one of these is a clean syntactic
// splice: α-rename the callee's locals into the caller's `VarId` space,
// substitute the call's argument atoms for the callee's parameters, drop the
// statements in place of the call, and bind the returned atom to the call's
// destination. Whole call frames disappear.
//
// **Works on the indirect call form the VM uses.** A call to a top-level def
// lowers to `Let(t, GlobalRef(g)); CallClosure(Var(t), args)` — an indirect call
// through the global holding the def's closure. This pass identifies the globals
// that hold a known capture-free closure (via `resolve::direct_call_targets`, the
// same rule the WASM-path resolver uses) and splices the *small, non-recursive*
// ones. Crucially it does **not** run `resolve_direct_calls`: that rewrites the
// call to `Call(Callee::Function)`, which the bytecode emitter turns into a fresh
// closure allocation per call (a VM pessimization on every *non*-inlined call).
// By matching the indirect form and leaving non-inlined calls untouched, this
// pass can only ever help. (The WASM backend resolves + monomorphizes itself.)
//
// **What is eligible.** A callee is inlined only when it is:
//   * synchronous (`!is_async`, not CPS-rewritten) — inlining across the async
//     state-machine boundary is not sound here;
//   * capture-free (the holding global's closure is, but we re-check);
//   * *straight-line with a single tail `Return`* — every statement before the
//     last is a `Let`/`Discard`, and the last is `Return(atom)`. A `when`/`if`
//     in tail position lowers to a `Match`/`If` with a `Return` in *each arm*
//     (see `lower::lower_when_tail`), so those bodies have multiple, nested
//     returns and are skipped — there is no goto/label form to splice them into
//     a structured caller without a result-var + labeled-break rewrite, which v1
//     doesn't attempt;
//   * small (`MAX_BODY_STMTS`); and
//   * not part of a call cycle among candidates (recursion would not terminate).
//
// **Behavior-neutral.** Argument atoms have no side effects and never allocate
// (the ANF invariant), so substituting one for a parameter at multiple use sites
// duplicates no work. The spliced statements run in the same order with the same
// values, so any runtime error fires identically (its source range now points at
// the callee's body — which is where the failing operation actually lives). A
// callee's trailing `TailCall` becomes a plain `CallClosure` once spliced into
// non-tail position; the orphaned `GlobalRef` temps are pruned. Validated
// end-to-end by the conformance gate: the VM oracle runs this pass, the deploy
// backends don't, and their outputs must still match.

use crate::resolve;
use crate::types::*;
use compiler::Range;
use std::collections::{HashMap, HashSet};

/// Max statements in a callee body (including the trailing `Return`) for it to
/// be inline-eligible. Keeps inlining to genuine one-liners / accessors.
const MAX_BODY_STMTS: usize = 8;

/// Recursion-depth guard for transitive inlining. The inlinable set is acyclic
/// (cyclic candidates are excluded), so this only backstops pathological
/// diamond-shaped blow-up; a generous value never trips for real code.
const MAX_DEPTH: u32 = 32;

/// A pre-extracted inlinable callee body: its parameters, the straight-line
/// statements before the tail `Return`, and the returned atom.
#[derive(Clone)]
struct InlineBody {
	params: Vec<VarId>,
	prelude: Vec<Stmt>,
	ret: Atom,
}

/// Splice small, non-recursive, directly-called functions into their call sites,
/// across the whole program. Idempotent.
pub fn inline(program: &mut IrProgram) {
	// Globals that hold a known capture-free, non-async closure: global index ->
	// the function it calls. The same rule `resolve` uses; here it tells us which
	// indirect `CallClosure(GlobalRef(g))` sites name a static target.
	let targets = resolve::direct_call_targets(program);
	if targets.is_empty() {
		return;
	}
	// 1. Shape-eligible candidates among those targets.
	let mut shaped: HashMap<FuncId, InlineBody> = HashMap::new();
	for fid in targets.values() {
		if shaped.contains_key(fid) {
			continue;
		}
		if let Some(f) = program.functions.get(fid.0 as usize) {
			if let Some(ib) = eligible_body(f) {
				shaped.insert(*fid, ib);
			}
		}
	}
	if shaped.is_empty() {
		return;
	}
	// 2. Drop candidates in a call cycle among candidates — inlining one would
	//    not terminate.
	let cyclic = cyclic_candidates(&shaped, &targets);
	let inlinable: HashMap<FuncId, InlineBody> = shaped
		.into_iter()
		.filter(|(fid, _)| !cyclic.contains(fid))
		.collect();
	if inlinable.is_empty() {
		return;
	}
	// 3. Splice eligible call sites in every function body, then prune the
	//    `GlobalRef` temps the splices orphaned.
	for f in &mut program.functions {
		let mut next_var = max_var_id(f) + 1;
		let mut var_globals: HashMap<u32, u32> = HashMap::new();
		let mut body = std::mem::replace(&mut f.body, Block(Vec::new()));
		inline_block(
			&mut body,
			&targets,
			&inlinable,
			&mut var_globals,
			&mut next_var,
			0,
		);
		f.body = body;
		resolve::prune_dead_global_refs(f);
	}
}

/// If `f` is a straight-line, single-tail-`Return`, capture-free, sync, small
/// function, return its inlinable form; else `None`.
fn eligible_body(f: &Function) -> Option<InlineBody> {
	if f.is_async || f.poll_fn.is_some() || !f.captures.is_empty() {
		return None;
	}
	let stmts = &f.body.0;
	if stmts.is_empty() || stmts.len() > MAX_BODY_STMTS {
		return None;
	}
	let (last, prelude) = stmts.split_last().unwrap();
	let StmtKind::Return(ret) = &last.kind else {
		return None;
	};
	for s in prelude {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if !rvalue_inlinable(rv) {
					return None;
				}
			}
			// Any control flow, `defer`, extra `Return`, `break`/`continue` makes
			// the body not straight-line-single-return.
			_ => return None,
		}
	}
	Some(InlineBody {
		params: f.params.clone(),
		prelude: prelude.to_vec(),
		ret: ret.clone(),
	})
}

/// An rvalue safe to lift into a caller. Only `Await` is rejected — it is the
/// async seam (and can't appear in a `!is_async` body anyway); everything else
/// (calls, allocations, field/element reads, arithmetic, …) splices cleanly.
fn rvalue_inlinable(rv: &Rvalue) -> bool {
	!matches!(rv, Rvalue::Await(_))
}

/// The set of candidates that can reach themselves through indirect calls to
/// other candidates — the cycles that would make inlining loop.
fn cyclic_candidates(
	shaped: &HashMap<FuncId, InlineBody>,
	targets: &HashMap<u32, FuncId>,
) -> HashSet<FuncId> {
	let mut edges: HashMap<FuncId, Vec<FuncId>> = HashMap::new();
	for (fid, ib) in shaped {
		// var -> global within this candidate's (straight-line) prelude.
		let mut vg: HashMap<u32, u32> = HashMap::new();
		for s in &ib.prelude {
			if let StmtKind::Let(v, Rvalue::GlobalRef(g)) = &s.kind {
				vg.insert(v.0, g.0);
			}
		}
		let mut outs = Vec::new();
		for s in &ib.prelude {
			let callee = match &s.kind {
				StmtKind::Let(_, Rvalue::CallClosure(Atom::Var(t), _))
				| StmtKind::Discard(Rvalue::CallClosure(Atom::Var(t), _)) => vg
					.get(&t.0)
					.and_then(|g| targets.get(g))
					.filter(|c| shaped.contains_key(c)),
				_ => None,
			};
			if let Some(c) = callee {
				outs.push(*c);
			}
		}
		edges.insert(*fid, outs);
	}
	shaped
		.keys()
		.copied()
		.filter(|&start| reaches_self(start, &edges))
		.collect()
}

/// Whether `start` is reachable from itself through `edges`.
fn reaches_self(start: FuncId, edges: &HashMap<FuncId, Vec<FuncId>>) -> bool {
	let mut stack: Vec<FuncId> = edges.get(&start).cloned().unwrap_or_default();
	let mut seen = HashSet::new();
	while let Some(n) = stack.pop() {
		if n == start {
			return true;
		}
		if !seen.insert(n) {
			continue;
		}
		if let Some(succ) = edges.get(&n) {
			stack.extend(succ.iter().copied());
		}
	}
	false
}

/// Inline every eligible indirect call site in `b`, recursing into nested blocks
/// and into freshly-spliced bodies (transitive inlining). `var_globals` maps
/// each var bound to a `GlobalRef` to that global's index; it is grown in source
/// order as the walk encounters `Let(_, GlobalRef)` bindings (including the ones
/// a splice introduces), so a `CallClosure(Var(t))` can be matched to its target.
fn inline_block(
	b: &mut Block,
	targets: &HashMap<u32, FuncId>,
	inlinable: &HashMap<FuncId, InlineBody>,
	var_globals: &mut HashMap<u32, u32>,
	next_var: &mut u32,
	depth: u32,
) {
	let mut out = Vec::with_capacity(b.0.len());
	for mut stmt in std::mem::take(&mut b.0) {
		let range = stmt.range;
		match &mut stmt.kind {
			StmtKind::Let(v, Rvalue::GlobalRef(g)) => {
				var_globals.insert(v.0, g.0);
				out.push(stmt);
			}
			StmtKind::Let(r, Rvalue::CallClosure(Atom::Var(t), args))
				if depth < MAX_DEPTH
					&& site_target(t.0, var_globals, targets, inlinable, args.len()).is_some() =>
			{
				let fid = site_target(t.0, var_globals, targets, inlinable, args.len()).unwrap();
				let (dest, args) = (*r, std::mem::take(args));
				splice(
					Some(dest),
					&inlinable[&fid],
					&args,
					targets,
					inlinable,
					var_globals,
					next_var,
					range,
					depth,
					&mut out,
				);
			}
			StmtKind::Discard(Rvalue::CallClosure(Atom::Var(t), args))
				if depth < MAX_DEPTH
					&& site_target(t.0, var_globals, targets, inlinable, args.len()).is_some() =>
			{
				let fid = site_target(t.0, var_globals, targets, inlinable, args.len()).unwrap();
				let args = std::mem::take(args);
				splice(
					None,
					&inlinable[&fid],
					&args,
					targets,
					inlinable,
					var_globals,
					next_var,
					range,
					depth,
					&mut out,
				);
			}
			StmtKind::If(_, t, e) => {
				inline_block(t, targets, inlinable, var_globals, next_var, depth);
				inline_block(e, targets, inlinable, var_globals, next_var, depth);
				out.push(stmt);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, blk) in arms.iter_mut() {
					inline_block(blk, targets, inlinable, var_globals, next_var, depth);
				}
				inline_block(default, targets, inlinable, var_globals, next_var, depth);
				out.push(stmt);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms.iter_mut() {
					inline_block(&mut arm.body, targets, inlinable, var_globals, next_var, depth);
				}
				out.push(stmt);
			}
			StmtKind::Loop(blk) => {
				inline_block(blk, targets, inlinable, var_globals, next_var, depth);
				out.push(stmt);
			}
			_ => out.push(stmt),
		}
	}
	b.0 = out;
}

/// If a `CallClosure(Var(t), ..)` with `argc` arguments names an inlinable target
/// of matching arity, return its `FuncId`.
fn site_target(
	t: u32,
	var_globals: &HashMap<u32, u32>,
	targets: &HashMap<u32, FuncId>,
	inlinable: &HashMap<FuncId, InlineBody>,
	argc: usize,
) -> Option<FuncId> {
	let g = var_globals.get(&t)?;
	let fid = *targets.get(g)?;
	let ib = inlinable.get(&fid)?;
	(ib.params.len() == argc).then_some(fid)
}

/// Splice `ib`'s renamed body into `out`, binding its result to `dest` (or
/// discarding it). `args` are substituted for the callee's parameters; the
/// callee's own locals are α-renamed to fresh `VarId`s above `next_var`.
#[allow(clippy::too_many_arguments)]
fn splice(
	dest: Option<VarId>,
	ib: &InlineBody,
	args: &[Atom],
	targets: &HashMap<u32, FuncId>,
	inlinable: &HashMap<FuncId, InlineBody>,
	var_globals: &mut HashMap<u32, u32>,
	next_var: &mut u32,
	range: Range,
	depth: u32,
	out: &mut Vec<Stmt>,
) {
	let mut subst: HashMap<u32, Atom> = HashMap::new();
	// Parameters -> the call's argument atoms.
	for (p, a) in ib.params.iter().zip(args) {
		subst.insert(p.0, a.clone());
	}
	// Every var the prelude defines -> a fresh caller var.
	for s in &ib.prelude {
		if let StmtKind::Let(v, _) = &s.kind {
			subst.entry(v.0).or_insert_with(|| {
				let fresh = VarId(*next_var);
				*next_var += 1;
				Atom::Var(fresh)
			});
		}
	}
	let mut spliced: Vec<Stmt> = ib.prelude.iter().map(|s| rename_stmt(s, &subst)).collect();
	if let Some(r) = dest {
		let ret = subst_atom(&ib.ret, &subst);
		spliced.push(Stmt::new(StmtKind::Let(r, Rvalue::Use(ret)), range));
	}
	// Transitively inline within the freshly-spliced body (its renamed
	// `GlobalRef` temps feed `var_globals` as `inline_block` scans them).
	let mut blk = Block(spliced);
	inline_block(&mut blk, targets, inlinable, var_globals, next_var, depth + 1);
	out.extend(blk.0);
}

/// Clone a prelude statement with its binding target and operand atoms renamed,
/// and a trailing `TailCall` (now in non-tail position) downgraded to a plain
/// `CallClosure`. The prelude only contains `Let`/`Discard`; other kinds are
/// cloned untouched defensively.
fn rename_stmt(s: &Stmt, subst: &HashMap<u32, Atom>) -> Stmt {
	let kind = match &s.kind {
		StmtKind::Let(v, rv) => {
			let nv = match subst.get(&v.0) {
				Some(Atom::Var(fresh)) => *fresh,
				_ => *v,
			};
			StmtKind::Let(nv, rename_rvalue(rv, subst))
		}
		StmtKind::Discard(rv) => StmtKind::Discard(rename_rvalue(rv, subst)),
		other => other.clone(),
	};
	Stmt::new(kind, s.range)
}

/// Clone an rvalue with every operand atom substituted; `TailCall` -> `CallClosure`.
fn rename_rvalue(rv: &Rvalue, subst: &HashMap<u32, Atom>) -> Rvalue {
	let a = |x: &Atom| subst_atom(x, subst);
	match rv {
		Rvalue::Use(x) => Rvalue::Use(a(x)),
		Rvalue::Bin(op, l, r) => Rvalue::Bin(*op, a(l), a(r)),
		Rvalue::Not(x) => Rvalue::Not(a(x)),
		Rvalue::Call(c, args) => Rvalue::Call(c.clone(), args.iter().map(a).collect()),
		// A tail call lifted into non-tail position is a plain closure call.
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			Rvalue::CallClosure(a(c), args.iter().map(a).collect())
		}
		Rvalue::GetDictMethod(x, i) => Rvalue::GetDictMethod(a(x), *i),
		Rvalue::MakeDict(args) => Rvalue::MakeDict(args.iter().map(a).collect()),
		Rvalue::MakeClosure(fid, caps) => Rvalue::MakeClosure(*fid, caps.iter().map(a).collect()),
		Rvalue::MakeRecord(fields) => {
			Rvalue::MakeRecord(fields.iter().map(|(n, x)| (n.clone(), a(x))).collect())
		}
		Rvalue::RecordUpdate { base, fields } => Rvalue::RecordUpdate {
			base: a(base),
			fields: fields.iter().map(|(n, x)| (n.clone(), a(x))).collect(),
		},
		Rvalue::GetField(x, n, sh) => Rvalue::GetField(a(x), n.clone(), sh.clone()),
		Rvalue::GetElement(x, i) => Rvalue::GetElement(a(x), *i),
		Rvalue::MakeVariant {
			enum_name,
			tag,
			payload,
		} => Rvalue::MakeVariant {
			enum_name: enum_name.clone(),
			tag: *tag,
			payload: payload.iter().map(a).collect(),
		},
		Rvalue::MakeVariantCtor { enum_name, tag } => Rvalue::MakeVariantCtor {
			enum_name: enum_name.clone(),
			tag: *tag,
		},
		Rvalue::Interpolate(args) => Rvalue::Interpolate(args.iter().map(a).collect()),
		Rvalue::GetTag(x) => Rvalue::GetTag(a(x)),
		Rvalue::GetPayload(x, i) => Rvalue::GetPayload(a(x), *i),
		Rvalue::MakeList(items) => Rvalue::MakeList(
			items
				.iter()
				.map(|it| match it {
					ListItem::Elem(x) => ListItem::Elem(a(x)),
					ListItem::Spread(x) => ListItem::Spread(a(x)),
				})
				.collect(),
		),
		Rvalue::MakeTuple(args) => Rvalue::MakeTuple(args.iter().map(a).collect()),
		Rvalue::GlobalRef(g) => Rvalue::GlobalRef(*g),
		Rvalue::Builtin(s) => Rvalue::Builtin(s.clone()),
		Rvalue::Await(x) => Rvalue::Await(a(x)),
		Rvalue::Box(x) => Rvalue::Box(a(x)),
		Rvalue::Unbox(x, r) => Rvalue::Unbox(a(x), *r),
	}
}

/// Apply the substitution to one atom. A `Var` maps to its substitute (an
/// argument atom for a parameter, a fresh `Var` for a local); anything not in
/// the map is left as-is (no such var should occur in a capture-free body).
fn subst_atom(x: &Atom, subst: &HashMap<u32, Atom>) -> Atom {
	match x {
		Atom::Var(v) => subst.get(&v.0).cloned().unwrap_or(Atom::Var(*v)),
		Atom::Const(c) => Atom::Const(c.clone()),
	}
}

/// The highest `VarId.0` appearing anywhere in `f` (params, captures, bindings,
/// pattern binds, and uses). Fresh inline locals are allocated above it.
fn max_var_id(f: &Function) -> u32 {
	let mut max = 0u32;
	for p in &f.params {
		max = max.max(p.0);
	}
	for c in &f.captures {
		max = max.max(c.0);
	}
	let mut note = |v: u32| max = max.max(v);
	each_var_block(&f.body, &mut note);
	max
}

fn each_var_block(b: &Block, note: &mut impl FnMut(u32)) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(v, rv) => {
				note(v.0);
				each_var_rvalue(rv, note);
			}
			StmtKind::Discard(rv) => each_var_rvalue(rv, note),
			StmtKind::If(c, t, e) => {
				each_var_atom(c, note);
				each_var_block(t, note);
				each_var_block(e, note);
			}
			StmtKind::Switch {
				scrutinee,
				arms,
				default,
			} => {
				each_var_atom(scrutinee, note);
				for (_, blk) in arms {
					each_var_block(blk, note);
				}
				each_var_block(default, note);
			}
			StmtKind::Match { subject, arms } => {
				each_var_atom(subject, note);
				for arm in arms {
					each_var_pattern(&arm.pattern, note);
					each_var_block(&arm.body, note);
				}
			}
			StmtKind::Loop(blk) => each_var_block(blk, note),
			StmtKind::Return(a) | StmtKind::PushDefer(a) => each_var_atom(a, note),
			StmtKind::Break | StmtKind::Continue | StmtKind::RunDefer(_) => {}
		}
	}
}

fn each_var_rvalue(rv: &Rvalue, note: &mut impl FnMut(u32)) {
	let mut a = |x: &Atom| each_var_atom(x, note);
	match rv {
		Rvalue::Use(x) | Rvalue::Not(x) | Rvalue::Box(x) | Rvalue::Unbox(x, _) => a(x),
		Rvalue::Bin(_, l, r) => {
			a(l);
			a(r);
		}
		Rvalue::Call(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => args.iter().for_each(a),
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			a(c);
			args.iter().for_each(a);
		}
		Rvalue::MakeRecord(fields) => fields.iter().for_each(|(_, x)| a(x)),
		Rvalue::RecordUpdate { base, fields } => {
			a(base);
			fields.iter().for_each(|(_, x)| a(x));
		}
		Rvalue::MakeVariant { payload, .. } => payload.iter().for_each(a),
		Rvalue::MakeList(items) => items.iter().for_each(|it| match it {
			ListItem::Elem(x) | ListItem::Spread(x) => a(x),
		}),
		Rvalue::GetDictMethod(x, _)
		| Rvalue::GetField(x, _, _)
		| Rvalue::GetElement(x, _)
		| Rvalue::GetTag(x)
		| Rvalue::GetPayload(x, _)
		| Rvalue::Await(x) => a(x),
		Rvalue::MakeVariantCtor { .. } | Rvalue::GlobalRef(_) | Rvalue::Builtin(_) => {}
	}
}

fn each_var_atom(x: &Atom, note: &mut impl FnMut(u32)) {
	if let Atom::Var(v) = x {
		note(v.0);
	}
}

fn each_var_pattern(p: &Pattern, note: &mut impl FnMut(u32)) {
	match p {
		Pattern::Bind(v) => note(v.0),
		Pattern::Wildcard | Pattern::Literal(_) => {}
		Pattern::Variant { fields, .. } => fields.iter().for_each(|f| each_var_pattern(f, note)),
		Pattern::Tuple(items) => items.iter().for_each(|f| each_var_pattern(f, note)),
		Pattern::List { items, rest } => {
			items.iter().for_each(|f| each_var_pattern(f, note));
			if let Some(ListRest::Bind(v)) = rest {
				note(v.0);
			}
		}
		Pattern::Record { fields, rest, .. } => {
			fields.iter().for_each(|(_, f)| each_var_pattern(f, note));
			if let RecordRest::Bind(v) = rest {
				note(v.0);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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

	/// A thunk whose body is `Let(0, MakeClosure(fid, [])); Return(0)` — the form
	/// `direct_call_targets` recognizes as "global holds a closure of `fid`".
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

	/// `Let(lv, GlobalRef(g)); Let(rv, CallClosure(Var(lv), args))` — the indirect
	/// call form lowering produces (the trailing `Return(rv)` is added by callers).
	fn indirect_call(global: u32, lv: u32, rv: u32, args: Vec<Atom>) -> Vec<Stmt> {
		vec![
			Stmt::new(
				StmtKind::Let(VarId(lv), Rvalue::GlobalRef(GlobalId(global))),
				syn(),
			),
			Stmt::new(
				StmtKind::Let(VarId(rv), Rvalue::CallClosure(Atom::Var(VarId(lv)), args)),
				syn(),
			),
		]
	}

	fn program(functions: Vec<Function>, globals: Vec<GlobalInit>, entry: u32) -> IrProgram {
		IrProgram {
			functions,
			globals,
			enums: Default::default(),
			entry: FuncId(entry),
			test_suites: vec![],
			test_new: None,
		}
	}

	fn has_call_closure(body: &[Stmt]) -> bool {
		body
			.iter()
			.any(|s| matches!(&s.kind, StmtKind::Let(_, Rvalue::CallClosure(..)) | StmtKind::Discard(Rvalue::CallClosure(..))))
	}

	// `def get = fun x { x.f }` inlined at an indirect call becomes a direct field
	// read on the substituted argument — the call (and its GlobalRef) are gone.
	#[test]
	fn inlines_accessor() {
		// fn0 = accessor: Let(1, GetField(Var0,"f")); Return(Var1)
		let accessor = boxed_fn(
			"get",
			vec![VarId(0)],
			vec![
				Stmt::new(
					StmtKind::Let(VarId(1), Rvalue::GetField(Atom::Var(VarId(0)), "f".into(), None)),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()),
			],
		);
		// fn1 = thunk; fn2 = caller(param Var7): indirect call of global 0 then return.
		let thunk = thunk_for(0);
		let mut caller_body = indirect_call(0, 8, 9, vec![Atom::Var(VarId(7))]);
		caller_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(9))), syn()));
		let caller = boxed_fn("caller", vec![VarId(7)], caller_body);
		let mut p = program(
			vec![accessor, thunk, caller],
			vec![GlobalInit::Thunk(FuncId(1))],
			2,
		);
		inline(&mut p);

		let body = &p.functions[2].body.0;
		assert!(!has_call_closure(body), "call should be inlined away: {body:?}");
		assert!(
			!body
				.iter()
				.any(|s| matches!(&s.kind, StmtKind::Let(_, Rvalue::GlobalRef(_)))),
			"orphaned GlobalRef should be pruned: {body:?}"
		);
		assert!(
			body.iter().any(|s| matches!(
				&s.kind,
				StmtKind::Let(_, Rvalue::GetField(Atom::Var(VarId(7)), name, _)) if name == "f"
			)),
			"expected GetField on the substituted arg Var(7): {body:?}"
		);
	}

	// A const argument substitutes straight into the body.
	#[test]
	fn substitutes_const_arg() {
		let inc = boxed_fn(
			"inc",
			vec![VarId(0)],
			vec![
				Stmt::new(
					StmtKind::Let(
						VarId(1),
						Rvalue::Bin(BinOp::AddInt, Atom::Var(VarId(0)), Atom::Const(Const::Int(1))),
					),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()),
			],
		);
		let thunk = thunk_for(0);
		let mut caller_body = indirect_call(0, 0, 1, vec![Atom::Const(Const::Int(41))]);
		caller_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()));
		let caller = boxed_fn("caller", vec![], caller_body);
		let mut p = program(vec![inc, thunk, caller], vec![GlobalInit::Thunk(FuncId(1))], 2);
		inline(&mut p);
		let body = &p.functions[2].body.0;
		assert!(
			body.iter().any(|s| matches!(
				&s.kind,
				StmtKind::Let(_, Rvalue::Bin(BinOp::AddInt, Atom::Const(Const::Int(41)), Atom::Const(Const::Int(1))))
			)),
			"const arg 41 should be substituted for the param: {body:?}"
		);
	}

	// A self-recursive callee (calls itself through its own global) is excluded
	// from the inlinable set, so the caller's call to it is left intact.
	#[test]
	fn leaves_recursion_alone() {
		// fn0 = rec: Let(1,GlobalRef(0)); Let(2,CallClosure(Var1,[Var0])); Return(2)
		let mut rec_body = indirect_call(0, 1, 2, vec![Atom::Var(VarId(0))]);
		rec_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(2))), syn()));
		let rec = boxed_fn("rec", vec![VarId(0)], rec_body);
		let thunk = thunk_for(0);
		let mut caller_body = indirect_call(0, 0, 1, vec![Atom::Const(Const::Int(3))]);
		caller_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()));
		let caller = boxed_fn("caller", vec![], caller_body);
		let mut p = program(vec![rec, thunk, caller], vec![GlobalInit::Thunk(FuncId(1))], 2);
		inline(&mut p);
		assert!(
			has_call_closure(&p.functions[2].body.0),
			"recursive callee must not be inlined: {:?}",
			p.functions[2].body.0
		);
	}

	// A `when`-bodied callee (a Match with a Return per arm) is not
	// straight-line-single-return, so it is left as a call.
	#[test]
	fn leaves_multi_return_alone() {
		let matchy = boxed_fn(
			"choose",
			vec![VarId(0)],
			vec![
				Stmt::new(
					StmtKind::Match {
						subject: Atom::Var(VarId(0)),
						arms: vec![MatchArm {
							pattern: Pattern::Wildcard,
							body: Block(vec![Stmt::new(
								StmtKind::Return(Atom::Const(Const::Int(0))),
								syn(),
							)]),
						}],
					},
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Const(Const::Unit)), syn()),
			],
		);
		let thunk = thunk_for(0);
		let mut caller_body = indirect_call(0, 0, 1, vec![Atom::Const(Const::Int(9))]);
		caller_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()));
		let caller = boxed_fn("caller", vec![], caller_body);
		let mut p = program(
			vec![matchy, thunk, caller],
			vec![GlobalInit::Thunk(FuncId(1))],
			2,
		);
		inline(&mut p);
		assert!(
			has_call_closure(&p.functions[2].body.0),
			"multi-return callee must not be inlined: {:?}",
			p.functions[2].body.0
		);
	}

	// Transitive: caller -> f -> g, both small leaves; both get spliced, no
	// CallClosure remains, and g's body threads the original const through f.
	#[test]
	fn inlines_transitively() {
		// fn0 = g: Let(1, Bin(MulInt, Var0, 2)); Return(Var1)
		let g = boxed_fn(
			"g",
			vec![VarId(0)],
			vec![
				Stmt::new(
					StmtKind::Let(
						VarId(1),
						Rvalue::Bin(BinOp::MulInt, Atom::Var(VarId(0)), Atom::Const(Const::Int(2))),
					),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()),
			],
		);
		// fn1 = f: indirect-calls global 0 (g) with its own param, returns it.
		let mut f_body = indirect_call(0, 1, 2, vec![Atom::Var(VarId(0))]);
		f_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(2))), syn()));
		let f = boxed_fn("f", vec![VarId(0)], f_body);
		// thunks: global 0 -> g (fn0), global 1 -> f (fn2 below). Lay out:
		// fn0=g, fn1=f, fn2=thunk(g)=thunk_for(0), fn3=thunk(f)=thunk_for(1), fn4=caller
		let thunk_g = thunk_for(0);
		let thunk_f = thunk_for(1);
		let mut caller_body = indirect_call(1, 0, 1, vec![Atom::Const(Const::Int(5))]);
		caller_body.push(Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()));
		let caller = boxed_fn("caller", vec![], caller_body);
		let mut p = program(
			vec![g, f, thunk_g, thunk_f, caller],
			// global 0 -> thunk_g (fn2), global 1 -> thunk_f (fn3)
			vec![GlobalInit::Thunk(FuncId(2)), GlobalInit::Thunk(FuncId(3))],
			4,
		);
		inline(&mut p);
		let body = &p.functions[4].body.0;
		assert!(
			!has_call_closure(body),
			"both calls should be inlined transitively: {body:?}"
		);
		assert!(
			body.iter().any(|s| matches!(
				&s.kind,
				StmtKind::Let(_, Rvalue::Bin(BinOp::MulInt, Atom::Const(Const::Int(5)), Atom::Const(Const::Int(2))))
			)),
			"g's body should be spliced with the arg threaded through f: {body:?}"
		);
	}

	// A `Discard` call site (effectful call, result unused) splices the body
	// without binding a result.
	#[test]
	fn inlines_discard_site() {
		let eff = boxed_fn(
			"eff",
			vec![VarId(0)],
			vec![
				Stmt::new(
					StmtKind::Let(
						VarId(1),
						Rvalue::Bin(BinOp::AddInt, Atom::Var(VarId(0)), Atom::Const(Const::Int(1))),
					),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Var(VarId(1))), syn()),
			],
		);
		let thunk = thunk_for(0);
		let caller = boxed_fn(
			"caller",
			vec![],
			vec![
				Stmt::new(
					StmtKind::Let(VarId(0), Rvalue::GlobalRef(GlobalId(0))),
					syn(),
				),
				Stmt::new(
					StmtKind::Discard(Rvalue::CallClosure(
						Atom::Var(VarId(0)),
						vec![Atom::Const(Const::Int(2))],
					)),
					syn(),
				),
				Stmt::new(StmtKind::Return(Atom::Const(Const::Unit)), syn()),
			],
		);
		let mut p = program(vec![eff, thunk, caller], vec![GlobalInit::Thunk(FuncId(1))], 2);
		inline(&mut p);
		let body = &p.functions[2].body.0;
		assert!(!has_call_closure(body), "discard call should be inlined: {body:?}");
		assert!(
			body.iter().any(|s| matches!(
				&s.kind,
				StmtKind::Let(_, Rvalue::Bin(BinOp::AddInt, Atom::Const(Const::Int(2)), _))
			)),
			"effect body should be spliced: {body:?}"
		);
	}
}
