// Fold boolean-literal `Match`es into a two-way `If`.
//
// `if cond { .. } else { .. }` lowers (via `lower_if`) to a `Match` on `cond`
// against the literal pattern `true`, with a wildcard else arm — the same shape a
// `when c is true { .. } is false { .. }` produces. A `Match` requires a *boxed*
// subject, so the repr/coercion pass boxes the (already-unboxed `i32`) condition
// into a heap `$bool` only for the matcher to read its payload straight back. That
// box+unbox round-trip lands in the hot path of every boolean `if`/`while` in the
// program.
//
// `StmtKind::If` already branches on an unboxed `i32` condition directly, and every
// downstream pass (loopify, reuse, repr, emit) handles it. This pass rewrites a
// `Match` whose arms are exactly boolean-literal/wildcard cases into that `If`, so
// the condition stays unboxed end to end. Runs before loopify (which loopifies
// `If`-dispatched tail recursion just as it does `Match`-dispatched).

use crate::types::*;

/// Rewrite boolean-literal matches into `If` across every function body.
pub fn fold_bool_matches(program: &mut IrProgram) {
	for f in &mut program.functions {
		fold_block(&mut f.body);
	}
}

fn fold_block(b: &mut Block) {
	for stmt in &mut b.0 {
		// Recurse into nested blocks first (post-order), then fold this statement.
		match &mut stmt.kind {
			StmtKind::If(_, t, e) => {
				fold_block(t);
				fold_block(e);
			}
			StmtKind::Loop(body) => fold_block(body),
			StmtKind::Switch { arms, default, .. } => {
				for (_, arm) in arms.iter_mut() {
					fold_block(arm);
				}
				fold_block(default);
			}
			StmtKind::Match { arms, .. } => {
				for arm in arms.iter_mut() {
					fold_block(&mut arm.body);
				}
			}
			_ => {}
		}
		if let StmtKind::Match { .. } = stmt.kind {
			let StmtKind::Match { subject, arms } = std::mem::replace(&mut stmt.kind, StmtKind::Break)
			else {
				unreachable!()
			};
			stmt.kind = match fold_one(subject, arms) {
				Ok(folded) => folded,
				Err((subject, arms)) => StmtKind::Match { subject, arms },
			};
		}
	}
}

/// True iff `p` matches the boolean literal `want`.
fn is_bool_lit(p: &Pattern, want: bool) -> bool {
	matches!(p, Pattern::Literal(Const::Bool(b)) if *b == want)
}

/// True iff `p` matches every remaining subject (`_` or a binding the bool case
/// never needs — only `Wildcard` here, since a `Bind` would drop its binding).
fn is_catch_all(p: &Pattern) -> bool {
	matches!(p, Pattern::Wildcard)
}

/// Try to rewrite a `Match` into an `If`. Returns `Ok(If)` on a recognized
/// boolean shape, or `Err` handing the pieces back unchanged.
fn fold_one(subject: Atom, mut arms: Vec<MatchArm>) -> Result<StmtKind, (Atom, Vec<MatchArm>)> {
	let empty = || Block(Vec::new());
	match arms.len() {
		// `if cond { t }` with no else: a single `true` arm, fall-through on false.
		1 if is_bool_lit(&arms[0].pattern, true) => {
			let t = arms.pop().unwrap().body;
			Ok(StmtKind::If(subject, t, empty()))
		}
		2 => {
			let (p0, p1) = (&arms[0].pattern, &arms[1].pattern);
			// `true => t`, then `_`/`false => e`.
			if is_bool_lit(p0, true) && (is_catch_all(p1) || is_bool_lit(p1, false)) {
				let mut it = arms.into_iter();
				let t = it.next().unwrap().body;
				let e = it.next().unwrap().body;
				Ok(StmtKind::If(subject, t, e))
			// `false => e`, then `_`/`true => t`.
			} else if is_bool_lit(p0, false) && (is_catch_all(p1) || is_bool_lit(p1, true)) {
				let mut it = arms.into_iter();
				let e = it.next().unwrap().body;
				let t = it.next().unwrap().body;
				Ok(StmtKind::If(subject, t, e))
			} else {
				Err((subject, arms))
			}
		}
		_ => Err((subject, arms)),
	}
}
