//! Record-shape monomorphization (WASM-only).
//!
//! A row-polymorphic or record-parameter function (`get-x = fun r { when r is {x:
//! v, ...} { v } }`) reads its param's fields by *name-scan* because the param's
//! shape isn't known in the single compiled body. This pass clones such a function
//! per call-site record shape, so the clone's param has a statically-known shape
//! and the read becomes a constant-index `struct.get` (and the caller passes the
//! record *nominal*, with no `lift`). The original stays for any call whose arg
//! shape isn't statically known (uniform fallback) — so this never regresses, it
//! only specializes the resolvable calls (the observed corpus is all direct-call
//! with a `MakeRecord` arg of known shape).
//!
//! It runs on the WASM-local clone of the `IrProgram`, after `resolve_direct_calls`
//! (so callees are visible as `Call(Callee::Function(fid), ..)`), and returns a map
//! from cloned `FuncId.0` to its params' nominal shapes. The emitter uses that to
//! (a) treat a clone's param as nominal (`scan::compute_nominal` seeds it) and (b)
//! pass a matching arg raw rather than lifting it. No `ir` types change: the clones
//! and call rewrites live in the cloned program, the shape metadata in the returned
//! side map.

use ir::{Atom, Block, Callee, IrProgram, Pattern, RecordShape, Rvalue, StmtKind};
use std::collections::HashMap;

/// Cap on distinct shape-specializations per source function — a code-size
/// bound. Calls beyond the cap keep the uniform (name-scan) path.
const MAX_CLONES_PER_FN: usize = 8;

/// A specialization key: the callee and, per param, the nominal shape it's
/// specialized to (`Some` to specialize, `None` to leave uniform).
type Key = (u32, Vec<Option<RecordShape>>);

/// Specialize record-parameter functions per call-site shape. Returns
/// `cloned FuncId.0 -> per-param nominal shapes`. Mutates `p` (adds clones, rewrites
/// the specialized calls). Idempotent in effect on a program that has none left.
pub(crate) fn specialize_record_shapes(
	p: &mut IrProgram,
) -> HashMap<u32, Vec<Option<RecordShape>>> {
	// 1. Candidate functions: a param read as a record in the body.
	let mut candidates: HashMap<u32, Vec<bool>> = HashMap::new();
	for (i, f) in p.functions.iter().enumerate() {
		let flags = candidate_params(f);
		if flags.iter().any(|&b| b) {
			candidates.insert(i as u32, flags);
		}
	}
	if candidates.is_empty() {
		return HashMap::new();
	}

	// 2. Discover specialization keys from every candidate call whose arg shapes are
	//    statically known (the arg is a `MakeRecord` in the caller). Assign each
	//    distinct key a fresh clone FuncId (appended sequentially after the existing
	//    functions), bounded per source.
	let n0 = p.functions.len();
	let mr_maps: Vec<HashMap<u32, RecordShape>> = p.functions.iter().map(makerecord_shapes).collect();
	let mut clones: HashMap<Key, u32> = HashMap::new();
	let mut specs: Vec<Key> = Vec::new();
	let mut per_src: HashMap<u32, usize> = HashMap::new();
	for fi in 0..n0 {
		collect_keys(
			&p.functions[fi].body,
			&candidates,
			&mr_maps[fi],
			&mut |key| {
				if clones.contains_key(&key) {
					return;
				}
				let cnt = per_src.entry(key.0).or_insert(0);
				if *cnt >= MAX_CLONES_PER_FN {
					return;
				}
				let new_fid = (n0 + specs.len()) as u32;
				clones.insert(key.clone(), new_fid);
				specs.push(key);
				*cnt += 1;
			},
		);
	}
	if specs.is_empty() {
		return HashMap::new();
	}

	// 3. Append the clones; record each clone's param shapes.
	let mut param_shapes: HashMap<u32, Vec<Option<RecordShape>>> = HashMap::new();
	for (idx, (src, shapes)) in specs.iter().enumerate() {
		let new_fid = (n0 + idx) as u32;
		let mut clone = p.functions[*src as usize].clone();
		clone.name = format!("{}$shape{}", clone.name, new_fid);
		param_shapes.insert(new_fid, shapes.clone());
		p.functions.push(clone);
		debug_assert_eq!(p.functions.len() as u32 - 1, new_fid);
	}

	// 4. Rewrite each specialized call to its clone (across all functions, clones
	//    included — a clone may itself contain a candidate call resolvable to an
	//    existing clone; new clones are *not* created here, so this is one pass).
	for fi in 0..p.functions.len() {
		let mr = makerecord_shapes(&p.functions[fi]);
		let mut body = std::mem::replace(&mut p.functions[fi].body, Block(Vec::new()));
		rewrite_calls(&mut body, &candidates, &mr, &clones);
		p.functions[fi].body = body;
	}
	param_shapes
}

/// Per-param "is this param read as a record in the body?" — it appears as a
/// `GetField` receiver or as a record-pattern `Match` subject. Such params benefit
/// from a nominal (struct.get) representation.
fn candidate_params(f: &ir::Function) -> Vec<bool> {
	let mut reads = std::collections::HashSet::new();
	record_reads(&f.body, &mut reads);
	f.params.iter().map(|p| reads.contains(&p.0)).collect()
}

fn record_reads(b: &Block, reads: &mut std::collections::HashSet<u32>) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Rvalue::GetField(Atom::Var(v), _, _) = rv {
					reads.insert(v.0);
				}
			}
			StmtKind::If(_, t, e) => {
				record_reads(t, reads);
				record_reads(e, reads);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					record_reads(b, reads);
				}
				record_reads(default, reads);
			}
			StmtKind::Match { subject, arms } => {
				if let Atom::Var(v) = subject {
					if arms
						.iter()
						.any(|a| matches!(a.pattern, Pattern::Record { .. }))
					{
						reads.insert(v.0);
					}
				}
				for a in arms {
					record_reads(&a.body, reads);
				}
			}
			StmtKind::Loop(b) => record_reads(b, reads),
			_ => {}
		}
	}
}

/// Map each `MakeRecord`-bound var in a function to its name-sorted shape — the
/// statically-known shapes of candidate-call args produced in that function.
fn makerecord_shapes(f: &ir::Function) -> HashMap<u32, RecordShape> {
	let mut m = HashMap::new();
	fn walk(b: &Block, m: &mut HashMap<u32, RecordShape>) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, Rvalue::MakeRecord(fields)) => {
					let mut names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
					names.sort();
					m.insert(v.0, RecordShape { fields: names });
				}
				StmtKind::If(_, t, e) => {
					walk(t, m);
					walk(e, m);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, b) in arms {
						walk(b, m);
					}
					walk(default, m);
				}
				StmtKind::Match { arms, .. } => {
					for a in arms {
						walk(&a.body, m);
					}
				}
				StmtKind::Loop(b) => walk(b, m),
				_ => {}
			}
		}
	}
	walk(&f.body, &mut m);
	m
}

/// The specialization key for a call to a candidate, or `None` if no param is
/// specializable (no statically-known record-shape arg in a candidate position).
fn key_for_call(
	callee: u32,
	args: &[Atom],
	candidates: &HashMap<u32, Vec<bool>>,
	mr: &HashMap<u32, RecordShape>,
) -> Option<Key> {
	let flags = candidates.get(&callee)?;
	let mut shapes: Vec<Option<RecordShape>> = vec![None; flags.len()];
	let mut any = false;
	for (i, want) in flags.iter().enumerate() {
		if !want {
			continue;
		}
		if let Some(Atom::Var(v)) = args.get(i) {
			if let Some(shape) = mr.get(&v.0) {
				shapes[i] = Some(shape.clone());
				any = true;
			}
		}
	}
	any.then_some((callee, shapes))
}

fn collect_keys(
	b: &Block,
	candidates: &HashMap<u32, Vec<bool>>,
	mr: &HashMap<u32, RecordShape>,
	emit: &mut impl FnMut(Key),
) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Rvalue::Call(Callee::Function(fid), args) | Rvalue::TailCallDirect(fid, args) = rv {
					if let Some(key) = key_for_call(fid.0, args, candidates, mr) {
						emit(key);
					}
				}
			}
			StmtKind::If(_, t, e) => {
				collect_keys(t, candidates, mr, emit);
				collect_keys(e, candidates, mr, emit);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_keys(b, candidates, mr, emit);
				}
				collect_keys(default, candidates, mr, emit);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_keys(&a.body, candidates, mr, emit);
				}
			}
			StmtKind::Loop(b) => collect_keys(b, candidates, mr, emit),
			_ => {}
		}
	}
}

fn rewrite_calls(
	b: &mut Block,
	candidates: &HashMap<u32, Vec<bool>>,
	mr: &HashMap<u32, RecordShape>,
	clones: &HashMap<Key, u32>,
) {
	for s in &mut b.0 {
		match &mut s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Rvalue::Call(Callee::Function(fid), args) | Rvalue::TailCallDirect(fid, args) = rv {
					if let Some(key) = key_for_call(fid.0, args, candidates, mr) {
						if let Some(&clone_fid) = clones.get(&key) {
							*fid = ir::FuncId(clone_fid);
						}
					}
				}
			}
			StmtKind::If(_, t, e) => {
				rewrite_calls(t, candidates, mr, clones);
				rewrite_calls(e, candidates, mr, clones);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					rewrite_calls(b, candidates, mr, clones);
				}
				rewrite_calls(default, candidates, mr, clones);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					rewrite_calls(&mut a.body, candidates, mr, clones);
				}
			}
			StmtKind::Loop(b) => rewrite_calls(b, candidates, mr, clones),
			_ => {}
		}
	}
}
