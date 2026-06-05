// The WASM async-lowering pass: turn `is_async` functions (with `Await` nodes, as
// lowering produces them) into a form the WasmGC backend can emit + drive.
//
// An `is_async` function arrives as a *step function* with an `Await`
// instruction and a heap-snapshotted frame — neither of which WasmGC can express
// (`ir::cps`'s module header explains why). So this pass:
//
//   1. runs `ir::cps::cps_transform`, which gives every awaiting function `f` an
//      Await-free *poll fn* sibling `pf = poll(state, resume) -> __poll` (an
//      ordinary IR function that lowers to wasm for free);
//   2. REPLACES `f`'s own (Await-bearing) body with a *task constructor*: calling
//      `f` now builds a cold `$task` of kind `async` carrying `f`'s poll closure
//      plus the initial CPS state (which `f` can build directly — it knows its
//      arg count). `f`'s Await-body is thus never emitted.
//
// The hand-emitted driver (`helpers/task.rs`) advances such a `$task` by calling
// its poll closure and interpreting the `__poll` result. The task primitives
// (`task.return`/…) and the side-effecting scope kernel are lowered in `emit.rs`.

use crate::runtime::task_kind;
use ir::{Atom, Block, Const, IrProgram, Rvalue, Stmt, StmtKind, VarId};

/// Synthetic enum name the async-fn constructor tags its `$task` with. `emit.rs`
/// special-cases `MakeVariant{enum_name == TASK_ENUM}` to build a `$task` struct.
pub(crate) const TASK_ENUM: &str = "__task";

/// Run the async lowering in place. Rewrites every awaiting function's body into a
/// `$task` constructor (over the poll fn `cps_transform` minted). A program with no
/// awaiting functions is left untouched — but every program is still driven by the
/// scheduler (the entry wrapper tolerates a plain-value `main`), so there's no
/// program-level "is async" distinction to report back.
pub(crate) fn lower(p: &mut IrProgram) {
	ir::cps::cps_transform(p);

	// Functions cps just rewrote to poll style (`poll_fn` set). Snapshot first;
	// the generated poll fns themselves have `poll_fn: None`, so they're skipped.
	let targets: Vec<usize> = p
		.functions
		.iter()
		.enumerate()
		.filter(|(_, f)| f.poll_fn.is_some())
		.map(|(i, _)| i)
		.collect();

	for i in targets {
		let f = &p.functions[i];
		let poll = f.poll_fn.unwrap();
		// Fresh VarIds above the function's params/captures (the body is replaced).
		let max = f
			.params
			.iter()
			.chain(f.captures.iter())
			.map(|v| v.0)
			.max()
			.unwrap_or(0);
		let pc = VarId(max + 1);
		let st = VarId(max + 2);
		let t = VarId(max + 3);

		// pc = the poll closure, capturing `f`'s captures (cps copies them).
		let caps: Vec<Atom> = f.captures.iter().map(|v| Atom::Var(*v)).collect();
		// state = { __tag: 0, __a0: p0, __a1: p1, … } — the CPS initial state, with
		// params seeded positionally (the field names `ir::cps` reads in segment 0).
		let mut fields: Vec<(String, Atom)> = vec![("__tag".to_string(), Atom::Const(Const::Int(0)))];
		for (idx, param) in f.params.iter().enumerate() {
			fields.push((format!("__a{idx}"), Atom::Var(*param)));
		}

		let body = Block(vec![
			Stmt::synthetic(StmtKind::Let(pc, Rvalue::MakeClosure(poll, caps))),
			Stmt::synthetic(StmtKind::Let(st, Rvalue::MakeRecord(fields))),
			Stmt::synthetic(StmtKind::Let(
				t,
				Rvalue::MakeVariant {
					enum_name: TASK_ENUM.to_string(),
					tag: task_kind::ASYNC as u32,
					payload: vec![Atom::Var(pc), Atom::Var(st)],
				},
			)),
			Stmt::synthetic(StmtKind::Return(Atom::Var(t))),
		]);

		let f = &mut p.functions[i];
		f.body = body;
		f.is_async = false;
		f.poll_fn = None;
	}
}
