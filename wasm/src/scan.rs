// IR pre-scans that run before body emission: the string-constant pool plus the
// walks that collect host-call tags, map locals to the builtin / variant-ctor
// they hold, and find zero-arg closures. All are read-only over the IR.

use crate::util::{EnumTable, variant_display};
use ir::{Atom, Block, Callee, Const, IrProgram, Rvalue, StmtKind};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct StrPool {
	pub(crate) bytes: Vec<u8>,
	pub(crate) at: HashMap<String, (u32, u32)>,
	pub(crate) bytes_at: HashMap<Vec<u8>, (u32, u32)>,
}

impl StrPool {
	pub(crate) fn intern(&mut self, s: &str) -> (u32, u32) {
		if let Some(&p) = self.at.get(s) {
			return p;
		}
		let off = self.bytes.len() as u32;
		self.bytes.extend_from_slice(s.as_bytes());
		let p = (off, s.len() as u32);
		self.at.insert(s.to_string(), p);
		p
	}

	/// Intern a raw byte sequence (a `bytes` literal — not necessarily UTF-8).
	pub(crate) fn intern_bytes(&mut self, b: &[u8]) -> (u32, u32) {
		if let Some(&p) = self.bytes_at.get(b) {
			return p;
		}
		let off = self.bytes.len() as u32;
		self.bytes.extend_from_slice(b);
		let p = (off, b.len() as u32);
		self.bytes_at.insert(b.to_vec(), p);
		p
	}
}

pub(crate) fn scan_strings(b: &Block, pool: &mut StrPool, enums: &EnumTable) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => scan_rvalue_strings(rv, pool, enums),
			StmtKind::Return(a) | StmtKind::PushDefer(a) => scan_atom_string(a, pool),
			StmtKind::If(_, t, e) => {
				scan_strings(t, pool, enums);
				scan_strings(e, pool, enums);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					scan_strings(b, pool, enums);
				}
				scan_strings(default, pool, enums);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					scan_pattern_names(&a.pattern, pool);
					scan_strings(&a.body, pool, enums);
				}
			}
			StmtKind::Loop(b) => scan_strings(b, pool, enums),
			_ => {}
		}
	}
}

pub(crate) fn scan_atom_string(a: &Atom, pool: &mut StrPool) {
	match a {
		Atom::Const(Const::Str(s)) => {
			pool.intern(s);
		}
		Atom::Const(Const::Bytes(b)) => {
			pool.intern_bytes(b);
		}
		_ => {}
	}
}

pub(crate) fn scan_rvalue_strings(rv: &Rvalue, pool: &mut StrPool, enums: &EnumTable) {
	for_each_atom(rv, &mut |a| scan_atom_string(a, pool));
	// Variant display names + record field names become `$str` constants (not atoms).
	match rv {
		Rvalue::MakeVariant { enum_name, tag, .. } => {
			pool.intern(&variant_display(enum_name, *tag, enums));
		}
		Rvalue::MakeVariantCtor { enum_name, tag } => {
			pool.intern(&variant_display(enum_name, *tag, enums));
		}
		_ => {}
	}
	// Record field names become `$str` constants too (not atoms).
	match rv {
		Rvalue::MakeRecord(fields, _) | Rvalue::RecordUpdate { fields, .. } => {
			for (n, _) in fields {
				pool.intern(n);
			}
		}
		Rvalue::GetField(_, name, _) => {
			pool.intern(name);
		}
		_ => {}
	}
}

/// Intern record-pattern field names (matched via `__getfield`, so they need
/// `$str` constants).
pub(crate) fn scan_pattern_names(p: &ir::Pattern, pool: &mut StrPool) {
	match p {
		ir::Pattern::Record { fields, .. } => {
			for (n, sub) in fields {
				pool.intern(n);
				scan_pattern_names(sub, pool);
			}
		}
		ir::Pattern::Variant { fields, .. } | ir::Pattern::Tuple(fields) => {
			fields.iter().for_each(|p| scan_pattern_names(p, pool))
		}
		ir::Pattern::List { items, .. } => items.iter().for_each(|p| scan_pattern_names(p, pool)),
		// String/bytes literal patterns (`when s is "digit"`) compare against an
		// interned `$str`/`$bytes` constant, so the pool must carry it.
		ir::Pattern::Literal(Const::Str(s)) => {
			pool.intern(s);
		}
		ir::Pattern::Literal(Const::Bytes(b)) => {
			pool.intern_bytes(b);
		}
		_ => {}
	}
}

/// Visit every `Atom` operand of an rvalue (exhaustive — so the string-constant
/// pre-scan never misses one, whatever the construct).
pub(crate) fn for_each_atom(rv: &Rvalue, f: &mut impl FnMut(&Atom)) {
	use ir::ListItem;
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
		| Rvalue::Await(a) => f(a),
		Rvalue::Bin(_, a, b) => {
			f(a);
			f(b);
		}
		Rvalue::Call(_, args)
		| Rvalue::TailCallDirect(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => args.iter().for_each(f),
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			f(c);
			args.iter().for_each(f);
		}
		Rvalue::MakeRecord(fields, _) => fields.iter().for_each(|(_, a)| f(a)),
		Rvalue::RecordUpdate { base, fields, .. } => {
			f(base);
			fields.iter().for_each(|(_, a)| f(a));
		}
		Rvalue::MakeVariant { payload, .. } => payload.iter().for_each(f),
		Rvalue::MakeList(items) => items.iter().for_each(|it| match it {
			ListItem::Elem(a) | ListItem::Spread(a) => f(a),
		}),
		Rvalue::MakeVariantCtor { .. } | Rvalue::GlobalRef(_) | Rvalue::Builtin(_) => {}
	}
}

/// Visit each builtin tag called (via a `GlobalRef`-to-builtin callee) in a block.
pub(crate) fn collect_host_calls(
	b: &Block,
	builtin_g: &HashMap<u32, String>,
	mut f: impl FnMut(&str),
) {
	// First map local vars to builtin tags within this block scope.
	let var_tags = builtin_var_tags(b, builtin_g);
	collect_inner(b, &var_tags, &mut f);
}

pub(crate) fn collect_inner(b: &Block, var_tags: &HashMap<u32, String>, f: &mut impl FnMut(&str)) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Some(tag) = callee_builtin_tag(rv, var_tags) {
					f(tag);
				}
			}
			StmtKind::If(_, t, e) => {
				collect_inner(t, var_tags, f);
				collect_inner(e, var_tags, f);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_inner(b, var_tags, f);
				}
				collect_inner(default, var_tags, f);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_inner(&a.body, var_tags, f);
				}
			}
			StmtKind::Loop(b) => collect_inner(b, var_tags, f),
			_ => {}
		}
	}
}

/// Map a function's local vars to the builtin tag they hold, from
/// `Let(v, GlobalRef(g))` where `g` is a `PreEvaluated(Builtin)`. Recurses into
/// nested blocks (a single var-id namespace per function).
pub(crate) fn builtin_var_tags(
	b: &Block,
	builtin_g: &HashMap<u32, String>,
) -> HashMap<u32, String> {
	let mut m = HashMap::new();
	fn walk(b: &Block, builtin_g: &HashMap<u32, String>, m: &mut HashMap<u32, String>) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, Rvalue::GlobalRef(g)) => {
					if let Some(tag) = builtin_g.get(&g.0) {
						m.insert(v.0, tag.clone());
					}
				}
				StmtKind::If(_, t, e) => {
					walk(t, builtin_g, m);
					walk(e, builtin_g, m);
				}
				StmtKind::Switch { arms, default, .. } => {
					for (_, b) in arms {
						walk(b, builtin_g, m);
					}
					walk(default, builtin_g, m);
				}
				StmtKind::Match { arms, .. } => {
					for a in arms {
						walk(&a.body, builtin_g, m);
					}
				}
				StmtKind::Loop(b) => walk(b, builtin_g, m),
				_ => {}
			}
		}
	}
	walk(b, builtin_g, &mut m);
	m
}

/// Map a function's local vars to the `(enum_name, variant tag)` they hold, from
/// `Let(v, MakeVariantCtor{..})`. Recurses into nested blocks.
pub(crate) fn ctor_var_tags(b: &Block) -> HashMap<u32, (String, u32)> {
	let mut m = HashMap::new();
	fn walk(b: &Block, m: &mut HashMap<u32, (String, u32)>) {
		for s in &b.0 {
			match &s.kind {
				StmtKind::Let(v, Rvalue::MakeVariantCtor { enum_name, tag }) => {
					m.insert(v.0, (enum_name.clone(), *tag));
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
	walk(b, &mut m);
	m
}

/// Decide which record-typed vars are built *nominal* (a `$shapeN` struct) instead
/// of the uniform `$record`: a `MakeRecord` whose result is *read as a record in
/// this function* — i.e. it appears as a `GetField` receiver or as the subject of a
/// `Match` with a record-pattern arm. Such a var's read becomes a constant-index
/// `struct.get`; a nominal value reaching any other (uniform-consuming) position is
/// `lift`ed to `$record` by `emit::atom`. A `MakeRecord` whose result never feeds a
/// local record read stays uniform (today's path), so a record built only to be
/// passed / stored / printed pays no nominal-build-then-lift overhead — that's what
/// keeps the open/row-poly path (e.g. the `record-access` benchmark) from
/// regressing pre-monomorphization. Returns `VarId.0 -> name-sorted shape`.
pub(crate) fn compute_nominal(
	f: &ir::Function,
	fid: u32,
	param_shapes: &HashMap<u32, Vec<Option<ir::RecordShape>>>,
	extra_nominal: &HashMap<u32, Vec<(u32, ir::RecordShape)>>,
) -> HashMap<u32, ir::RecordShape> {
	let mut nominal = HashMap::new();
	// (a) This function's own nominal params (from record-shape monomorphization):
	// a specialized clone's param holds a `$shapeN` at runtime, so reads on it are
	// `struct.get`.
	if let Some(shapes) = param_shapes.get(&fid) {
		for (i, sh) in shapes.iter().enumerate() {
			if let (Some(sh), Some(p)) = (sh, f.params.get(i)) {
				nominal.insert(p.0, sh.clone());
			}
		}
	}
	// (a') Extra nominal vars the substitution-driven engine recorded — a specialized
	// lambda's captures of a nominal var (e.g. a `list.fold` lambda capturing the
	// enclosing record param). Their runtime value is the captured `$shapeN`.
	if let Some(extras) = extra_nominal.get(&fid) {
		for (v, sh) in extras {
			nominal.insert(*v, sh.clone());
		}
	}
	// (b) A `MakeRecord` with a typed shape is always built nominal (it flows
	// uniformly as a `$value` and self-lifts at generic consumers). A `None`-shape
	// `MakeRecord` stays uniform, except one passed as an arg into a nominal callee
	// param (record-shape monomorphization) — `read` collects those so they're built
	// with the boxed shape the param expects.
	let mut read = HashSet::new();
	collect_nominal_param_args(&f.body, param_shapes, &mut read);
	collect_nominal_records(&f.body, &read, &mut nominal);
	nominal
}

/// Add to `read` any `MakeRecord` arg var passed into a *nominal* callee param
/// (`Call(Callee::Function(fid), ..)` with `param_shapes[fid][i] = Some`), so it is
/// built nominal and passed raw at the call.
fn collect_nominal_param_args(
	b: &Block,
	param_shapes: &HashMap<u32, Vec<Option<ir::RecordShape>>>,
	read: &mut HashSet<u32>,
) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, rv) | StmtKind::Discard(rv) => {
				if let Rvalue::Call(Callee::Function(fid), args) | Rvalue::TailCallDirect(fid, args) = rv {
					if let Some(shapes) = param_shapes.get(&fid.0) {
						for (i, sh) in shapes.iter().enumerate() {
							if sh.is_some() {
								if let Some(Atom::Var(v)) = args.get(i) {
									read.insert(v.0);
								}
							}
						}
					}
				}
			}
			StmtKind::If(_, t, e) => {
				collect_nominal_param_args(t, param_shapes, read);
				collect_nominal_param_args(e, param_shapes, read);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_nominal_param_args(b, param_shapes, read);
				}
				collect_nominal_param_args(default, param_shapes, read);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_nominal_param_args(&a.body, param_shapes, read);
				}
			}
			StmtKind::Loop(b) => collect_nominal_param_args(b, param_shapes, read),
			_ => {}
		}
	}
}

/// Mark the nominal record producers whose result var is in `read`: a
/// `MakeRecord` (shape = its name-sorted fields), and a `RecordUpdate` on an
/// already-nominal base (shape-preserving, so it inherits the base's shape — built
/// as a `struct.new` copy rather than the uniform helper). A forward walk, so a
/// `RecordUpdate`'s base (bound earlier in ANF) is already in `out`.
fn collect_nominal_records(
	b: &Block,
	read: &HashSet<u32>,
	out: &mut HashMap<u32, ir::RecordShape>,
) {
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(v, Rvalue::MakeRecord(fields, shape)) => {
				match shape {
					// A `MakeRecord` carrying a *typed* shape is built nominal
					// unconditionally — not just when read as a record in this function. A
					// nominal record is a `$value` subtype, so it flows uniformly everywhere
					// a boxed value goes (`emit::atom` no longer lifts it); the generic
					// consumers self-lift it via `__denominalize`. Keeping a record nominal
					// as it's stored into a list and read back is what lets field access
					// after `list.get` stay a constant-index `struct.get` once the reader is
					// monomorphic.
					Some(shape) => {
						out.insert(v.0, shape.clone());
					}
					// A `None`-shape `MakeRecord` (a synthetic record built without a type —
					// e.g. the CPS poll-state) has no closed shape, so it stays uniform and
					// the async-runtime helpers that read it by name keep seeing a `$record`
					// — *unless* it flows into a record-shape-monomorphized callee's nominal
					// param (in `read`), where the boxed shape the param expects must match.
					None if read.contains(&v.0) => {
						out.insert(
							v.0,
							ir::RecordShape::boxed_from_names(fields.iter().map(|(n, _)| n.clone()).collect()),
						);
					}
					None => {}
				}
			}
			// `{ ...base, f: v }` is shape-preserving, so it inherits `base`'s shape and
			// is built nominal exactly when `base` is (a `struct.new` copy). The forward
			// walk means `base` (bound earlier in ANF) is already recorded.
			StmtKind::Let(
				v,
				Rvalue::RecordUpdate {
					base: Atom::Var(b), ..
				},
			) => {
				if let Some(shape) = out.get(&b.0).cloned() {
					out.insert(v.0, shape);
				}
			}
			StmtKind::If(_, t, e) => {
				collect_nominal_records(t, read, out);
				collect_nominal_records(e, read, out);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_nominal_records(b, read, out);
				}
				collect_nominal_records(default, read, out);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_nominal_records(&a.body, read, out);
				}
			}
			StmtKind::Loop(b) => collect_nominal_records(b, read, out),
			_ => {}
		}
	}
}

/// Collect `MakeClosure` targets that have zero IR params (the `fun { }` form,
/// typed `nothing -> a` — arity 1 at every call site).
pub(crate) fn collect_zero_arg_closures(b: &Block, p: &IrProgram, out: &mut HashSet<u32>) {
	fn rv(rv: &Rvalue, p: &IrProgram, out: &mut HashSet<u32>) {
		if let Rvalue::MakeClosure(fid, _) = rv {
			if p.functions[fid.0 as usize].params.is_empty() {
				out.insert(fid.0);
			}
		}
	}
	for s in &b.0 {
		match &s.kind {
			StmtKind::Let(_, r) | StmtKind::Discard(r) => rv(r, p, out),
			StmtKind::If(_, t, e) => {
				collect_zero_arg_closures(t, p, out);
				collect_zero_arg_closures(e, p, out);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_zero_arg_closures(b, p, out);
				}
				collect_zero_arg_closures(default, p, out);
			}
			StmtKind::Match { arms, .. } => {
				for a in arms {
					collect_zero_arg_closures(&a.body, p, out);
				}
			}
			StmtKind::Loop(b) => collect_zero_arg_closures(b, p, out),
			_ => {}
		}
	}
}

pub(crate) fn callee_builtin_tag<'a>(
	rv: &'a Rvalue,
	var_tags: &'a HashMap<u32, String>,
) -> Option<&'a str> {
	match rv {
		// A builtin resolved to a typed call node (`resolve_builtins`) carries its
		// tag directly. The legacy indirect form (a builtin global loaded into a var,
		// then called) is recovered through `var_tags`.
		Rvalue::Call(Callee::Builtin(tag, _), _) => Some(tag.as_str()),
		Rvalue::CallClosure(Atom::Var(v), _) | Rvalue::TailCall(Atom::Var(v), _) => {
			var_tags.get(&v.0).map(|s| s.as_str())
		}
		_ => None,
	}
}

/// True if `block` (or any nested block) schedules a `defer` — i.e. contains a
/// `PushDefer`. Drives the emitter's per-function cleanup-list allocation: a
/// defer-free function pays nothing.
pub(crate) fn block_has_pushdefer(block: &Block) -> bool {
	block.0.iter().any(|s| match &s.kind {
		StmtKind::PushDefer(_) => true,
		StmtKind::If(_, t, e) => block_has_pushdefer(t) || block_has_pushdefer(e),
		StmtKind::Switch { arms, default, .. } => {
			arms.iter().any(|(_, b)| block_has_pushdefer(b)) || block_has_pushdefer(default)
		}
		StmtKind::Match { arms, .. } => arms.iter().any(|a| block_has_pushdefer(&a.body)),
		StmtKind::Loop(b) => block_has_pushdefer(b),
		_ => false,
	})
}
