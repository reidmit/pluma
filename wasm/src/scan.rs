// IR pre-scans that run before body emission: the string-constant pool plus the
// walks that collect host-call tags, map locals to the builtin / variant-ctor
// they hold, and find zero-arg closures. All are read-only over the IR.

use std::collections::{HashMap, HashSet};

use ir::{Atom, Block, Const, IrProgram, Rvalue, StmtKind};

use crate::util::{variant_display, EnumTable};

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
		Rvalue::MakeRecord(fields) | Rvalue::RecordUpdate { fields, .. } => {
			for (n, _) in fields {
				pool.intern(n);
			}
		}
		Rvalue::GetField(_, name) => {
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
		| Rvalue::GetField(a, _)
		| Rvalue::GetElement(a, _)
		| Rvalue::GetTag(a)
		| Rvalue::GetPayload(a, _)
		| Rvalue::Await(a) => f(a),
		Rvalue::Bin(_, a, b) => {
			f(a);
			f(b);
		}
		Rvalue::Call(_, args)
		| Rvalue::MakeDict(args)
		| Rvalue::MakeTuple(args)
		| Rvalue::Interpolate(args)
		| Rvalue::MakeClosure(_, args) => args.iter().for_each(f),
		Rvalue::CallClosure(c, args) | Rvalue::TailCall(c, args) => {
			f(c);
			args.iter().for_each(f);
		}
		Rvalue::MakeRecord(fields) => fields.iter().for_each(|(_, a)| f(a)),
		Rvalue::RecordUpdate { base, fields } => {
			f(base);
			fields.iter().for_each(|(_, a)| f(a));
		}
		Rvalue::MakeVariant { payload, .. } => payload.iter().for_each(f),
		Rvalue::MakeList(items) => items.iter().for_each(|it| match it {
			ListItem::Elem(a) | ListItem::Spread(a) => f(a),
		}),
		Rvalue::MakeVariantCtor { .. }
		| Rvalue::Regex(_)
		| Rvalue::GlobalRef(_)
		| Rvalue::Builtin(_) => {}
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
	rv: &Rvalue,
	var_tags: &'a HashMap<u32, String>,
) -> Option<&'a str> {
	let callee = match rv {
		Rvalue::CallClosure(c, _) | Rvalue::TailCall(c, _) => c,
		_ => return None,
	};
	if let Atom::Var(v) = callee {
		var_tags.get(&v.0).map(|s| s.as_str())
	} else {
		None
	}
}
