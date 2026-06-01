// IR -> JavaScript lowering: the third consumer of `ir::IrProgram` (after the
// bytecode VM via `codegen` and the WasmGC backend via `wasm`). Targets the
// browser/client.
//
// The IR is ANF + structured control flow and JS is uniformly boxed (like the
// VM), so this is a mechanical, mostly 1:1 translation: each `Let(v, rv)` becomes
// `_v = <expr>;`, each `Stmt` a JS statement. Repr/`Box`/`Unbox` are no-ops here
// (JS values are already tagged), exactly as the bytecode emitter treats them.
//
// Synchronous core only: `Await`/CPS and the async/wire builtins are out of
// scope for this milestone (the runner errors loudly if one is reached).

use std::collections::HashMap;
use std::fmt::Write;

use ir::{
	Atom, BinOp, Block, Callee, Const, Function, GlobalInit, IrProgram, ListItem, ListRest, Pattern,
	PreEval, RecordRest, Rvalue, StmtKind, VarId,
};

/// Lower a complete IR program to a self-contained JavaScript module (runtime
/// preamble + compiled functions + global table + entry).
pub fn emit(program: &IrProgram) -> Result<String, String> {
	let mut e = Emitter::new(program);
	let mut out = String::new();
	out.push_str(RUNTIME);
	out.push_str("\n// ===== compiled functions =====\n");
	for (i, f) in program.functions.iter().enumerate() {
		e.emit_function(i, f, &mut out)?;
	}
	e.emit_globals(&mut out)?;
	e.emit_enum_bindings(&mut out);
	writeln!(
		out,
		"globalThis.__plumaResult = __run(() => $f{}(null));",
		program.entry.0
	)
	.unwrap();
	Ok(out)
}

/// The runtime preamble (value model, native builtins, the program runner),
/// prepended verbatim to every emitted module.
const RUNTIME: &str = include_str!("runtime.js");

struct Emitter<'a> {
	program: &'a IrProgram,
	/// qualified-enum-name -> [(variant_name, arity)].
	enums: &'a HashMap<String, Vec<(String, usize)>>,
	/// Fresh-temp counter for match subjects / loop labels.
	tmp: u32,
	/// Active loop labels (innermost last), so `Break`/`Continue` target the loop
	/// itself rather than a nested `switch`.
	loop_labels: Vec<String>,
	/// Whether tail calls in the function currently being emitted may lower to a
	/// `__TC` bounce. Disabled for defer-bearing functions: their body runs inside
	/// a `try`, and a bounce would escape it unevaluated, so the `finally` cleanup
	/// would run *before* the tail call's effect. Mirrors the VM's TCO-downgrade
	/// when a frame has pending defers.
	tco: bool,
}

impl<'a> Emitter<'a> {
	fn new(program: &'a IrProgram) -> Self {
		Emitter {
			program,
			enums: &program.enums,
			tmp: 0,
			loop_labels: Vec::new(),
			tco: true,
		}
	}

	fn variant_name(&self, enum_name: &str, tag: u32) -> Result<&str, String> {
		self
			.enums
			.get(enum_name)
			.and_then(|vs| vs.get(tag as usize))
			.map(|(n, _)| n.as_str())
			.ok_or_else(|| format!("js: unknown variant {enum_name}#{tag}"))
	}

	fn variant_arity(&self, enum_name: &str, tag: u32) -> Result<usize, String> {
		self
			.enums
			.get(enum_name)
			.and_then(|vs| vs.get(tag as usize))
			.map(|(_, a)| *a)
			.ok_or_else(|| format!("js: unknown variant {enum_name}#{tag}"))
	}

	// ---- functions --------------------------------------------------------

	fn emit_function(&mut self, idx: usize, f: &Function, out: &mut String) -> Result<(), String> {
		// Signature: every compiled function takes the capture env first, then the
		// Pluma params. `__env` is null for top-level / directly-called functions.
		let params: Vec<String> = f.params.iter().map(|p| var(*p)).collect();
		write!(out, "function $f{idx}(__env").unwrap();
		for p in &params {
			write!(out, ", {p}").unwrap();
		}
		out.push_str(") {\n");

		// Captures: bind each to its env slot.
		for (i, c) in f.captures.iter().enumerate() {
			writeln!(out, "\t{} = __env[{i}];", decl_var(*c)).unwrap();
		}

		// Hoist a `let` for every let-/pattern-bound var (mirrors the VM's flat
		// per-frame slots): assignments below never re-declare, so a var written in
		// one branch/arm is visible after the join.
		let mut bound = Vec::new();
		collect_bound(&f.body, &mut bound);
		// captures are declared above (as `let` via decl_var); params are JS params.
		// Dedup: the IR can reuse a `VarId` across mutually-exclusive branches/arms
		// (one VM slot), but JS allows a given `let` only once.
		let param_set: std::collections::HashSet<u32> = f.params.iter().map(|v| v.0).collect();
		let cap_set: std::collections::HashSet<u32> = f.captures.iter().map(|v| v.0).collect();
		let mut seen = std::collections::HashSet::new();
		for v in &bound {
			if !param_set.contains(v) && !cap_set.contains(v) && seen.insert(*v) {
				writeln!(out, "\tlet _{v};").unwrap();
			}
		}

		let has_defers = block_has_defer(&f.body);
		if has_defers {
			out.push_str("\tconst __defers = [];\n\ttry {\n");
		}
		// Defer-bearing bodies run inside a `try`; a tail-call bounce would escape
		// it before executing, reordering effects past the `finally` cleanup. So
		// downgrade TCO here (real landed call), exactly as the VM does.
		self.tco = !has_defers;
		let mut body = String::new();
		self.emit_block(&f.body, &mut body, if has_defers { 2 } else { 1 })?;
		out.push_str(&body);
		if has_defers {
			out.push_str(
				"\t} finally {\n\t\tfor (let __i = __defers.length - 1; __i >= 0; __i--) __land(__defers[__i]());\n\t}\n",
			);
		}
		out.push_str("}\n");
		Ok(())
	}

	fn emit_block(&mut self, block: &Block, out: &mut String, ind: usize) -> Result<(), String> {
		for stmt in &block.0 {
			self.emit_stmt(&stmt.kind, out, ind)?;
		}
		Ok(())
	}

	fn emit_stmt(&mut self, kind: &StmtKind, out: &mut String, ind: usize) -> Result<(), String> {
		let pad = "\t".repeat(ind);
		match kind {
			StmtKind::Let(v, rv) => {
				let expr = self.rvalue(rv)?;
				writeln!(out, "{pad}_{} = {expr};", v.0).unwrap();
			}
			StmtKind::Discard(rv) => {
				let expr = self.rvalue(rv)?;
				writeln!(out, "{pad}{expr};").unwrap();
			}
			StmtKind::Return(a) => {
				writeln!(out, "{pad}return {};", self.atom(a)).unwrap();
			}
			StmtKind::If(cond, then_b, else_b) => {
				writeln!(out, "{pad}if ({}) {{", self.atom(cond)).unwrap();
				self.emit_block(then_b, out, ind + 1)?;
				if !else_b.0.is_empty() {
					writeln!(out, "{pad}}} else {{").unwrap();
					self.emit_block(else_b, out, ind + 1)?;
				}
				writeln!(out, "{pad}}}").unwrap();
			}
			StmtKind::Switch {
				scrutinee,
				arms,
				default,
			} => {
				writeln!(out, "{pad}switch ({}) {{", self.atom(scrutinee)).unwrap();
				for (k, b) in arms {
					writeln!(out, "{pad}case {k}: {{").unwrap();
					self.emit_block(b, out, ind + 2)?;
					writeln!(out, "{pad}break; }}").unwrap();
				}
				writeln!(out, "{pad}default: {{").unwrap();
				self.emit_block(default, out, ind + 2)?;
				writeln!(out, "{pad}}} }}").unwrap();
			}
			StmtKind::Loop(b) => {
				let label = self.fresh("L");
				writeln!(out, "{pad}{label}: while (true) {{").unwrap();
				// Push the loop label so Break/Continue target it (a plain `break`
				// inside a nested `switch` would otherwise escape only the switch).
				self.loop_labels.push(label);
				self.emit_block(b, out, ind + 1)?;
				self.loop_labels.pop();
				writeln!(out, "{pad}}}").unwrap();
			}
			StmtKind::Break => {
				let l = self.loop_labels.last().ok_or("js: break outside loop")?;
				writeln!(out, "{pad}break {l};").unwrap();
			}
			StmtKind::Continue => {
				let l = self.loop_labels.last().ok_or("js: continue outside loop")?;
				writeln!(out, "{pad}continue {l};").unwrap();
			}
			StmtKind::PushDefer(closure) => {
				writeln!(out, "{pad}__defers.push({});", self.atom(closure)).unwrap();
			}
			StmtKind::Match { subject, arms } => {
				let s = self.fresh("__s");
				let label = self.fresh("__m");
				writeln!(out, "{pad}const {s} = {};", self.atom(subject)).unwrap();
				writeln!(out, "{pad}{label}: {{").unwrap();
				for arm in arms {
					let test = self.pattern_test(&arm.pattern, &s);
					writeln!(out, "{pad}\tif ({test}) {{").unwrap();
					let mut binds = String::new();
					self.pattern_binds(&arm.pattern, &s, &mut binds, ind + 2);
					out.push_str(&binds);
					self.emit_block(&arm.body, out, ind + 2)?;
					writeln!(out, "{pad}\t\tbreak {label};").unwrap();
					writeln!(out, "{pad}\t}}").unwrap();
				}
				writeln!(out, "{pad}}}").unwrap();
			}
			StmtKind::RunDefer(_) => {
				return Err("js: RunDefer is CPS-only; sync defer uses the try/finally path".into());
			}
		}
		Ok(())
	}

	// ---- pattern compilation ---------------------------------------------

	/// A boolean JS expression that is true iff `path` matches `pat` (no binding).
	fn pattern_test(&self, pat: &Pattern, path: &str) -> String {
		match pat {
			Pattern::Wildcard | Pattern::Bind(_) => "true".to_string(),
			Pattern::Literal(c) => match c {
				Const::Int(n) => format!("{path} === {n}"),
				Const::Bool(b) => format!("{path} === {b}"),
				Const::Str(s) => format!("{path} === {}", js_str(s)),
				Const::Float(f) => format!("{path} instanceof PFloat && {path}.v === {}", js_float(*f)),
				Const::Bytes(b) => format!("__eq({path}, {})", js_bytes(b)),
				Const::Unit => format!("{path} === NOTHING"),
				Const::Duration(n) => format!("{path} instanceof PDuration && {path}.ns === {n}n"),
			},
			Pattern::Variant { variant, fields } => {
				let mut t = format!("{path}.name === {}", js_str(variant));
				for (i, f) in fields.iter().enumerate() {
					let sub = self.pattern_test(f, &format!("{path}.p[{i}]"));
					if sub != "true" {
						write!(t, " && {sub}").unwrap();
					}
				}
				t
			}
			Pattern::Tuple(elems) => {
				let mut parts = Vec::new();
				for (i, e) in elems.iter().enumerate() {
					let sub = self.pattern_test(e, &format!("{path}.e[{i}]"));
					if sub != "true" {
						parts.push(sub);
					}
				}
				if parts.is_empty() {
					"true".to_string()
				} else {
					parts.join(" && ")
				}
			}
			Pattern::List { items, rest } => {
				let mut parts = Vec::new();
				if rest.is_some() {
					parts.push(format!("{path}.length >= {}", items.len()));
				} else {
					parts.push(format!("{path}.length === {}", items.len()));
				}
				for (i, it) in items.iter().enumerate() {
					let sub = self.pattern_test(it, &format!("{path}[{i}]"));
					if sub != "true" {
						parts.push(sub);
					}
				}
				parts.join(" && ")
			}
			Pattern::Record { fields, rest, .. } => {
				let mut parts = Vec::new();
				if matches!(rest, RecordRest::Exact) {
					parts.push(format!("Object.keys({path}).length === {}", fields.len()));
				}
				for (name, p) in fields {
					let sub = self.pattern_test(p, &format!("{path}[{}]", js_str(name)));
					if sub != "true" {
						parts.push(sub);
					}
				}
				if parts.is_empty() {
					"true".to_string()
				} else {
					parts.join(" && ")
				}
			}
		}
	}

	/// Emit the binding assignments for a matched pattern (the test already passed).
	fn pattern_binds(&self, pat: &Pattern, path: &str, out: &mut String, ind: usize) {
		let pad = "\t".repeat(ind);
		match pat {
			Pattern::Wildcard | Pattern::Literal(_) => {}
			Pattern::Bind(v) => writeln!(out, "{pad}_{} = {path};", v.0).unwrap(),
			Pattern::Variant { fields, .. } => {
				for (i, f) in fields.iter().enumerate() {
					self.pattern_binds(f, &format!("{path}.p[{i}]"), out, ind);
				}
			}
			Pattern::Tuple(elems) => {
				for (i, e) in elems.iter().enumerate() {
					self.pattern_binds(e, &format!("{path}.e[{i}]"), out, ind);
				}
			}
			Pattern::List { items, rest } => {
				for (i, it) in items.iter().enumerate() {
					self.pattern_binds(it, &format!("{path}[{i}]"), out, ind);
				}
				if let Some(ListRest::Bind(v)) = rest {
					writeln!(out, "{pad}_{} = {path}.slice({});", v.0, items.len()).unwrap();
				}
			}
			Pattern::Record { fields, rest, .. } => {
				for (name, p) in fields {
					self.pattern_binds(p, &format!("{path}[{}]", js_str(name)), out, ind);
				}
				if let RecordRest::Bind(v) = rest {
					let excluded: Vec<String> = fields.iter().map(|(n, _)| js_str(n)).collect();
					writeln!(
						out,
						"{pad}_{} = __recordRest({path}, [{}]);",
						v.0,
						excluded.join(", ")
					)
					.unwrap();
				}
			}
		}
	}

	// ---- rvalues / atoms --------------------------------------------------

	fn rvalue(&self, rv: &Rvalue) -> Result<String, String> {
		Ok(match rv {
			Rvalue::Use(a) => self.atom(a),
			Rvalue::Box(a) | Rvalue::Unbox(a, _) => self.atom(a),
			Rvalue::Bin(op, a, b) => self.binop(*op, a, b),
			Rvalue::Not(a) => format!("(!{})", self.atom(a)),
			Rvalue::Call(callee, args) => self.call(callee, args)?,
			// A non-tail closure call: land it, since the callee's body may itself
			// end in a tail call and hand back a `__TC` bounce.
			Rvalue::CallClosure(c, args) => format!("__land({}({}))", self.atom(c), self.atoms(args)),
			// A tail call doesn't call — it hands back a `__TC` bounce that the
			// consuming boundary (`__land`) loops flat, so the chain stays O(1)
			// host frames. The IR always immediately `Return`s this value, so the
			// caller of *this* function lands it. In a defer-bearing function TCO is
			// downgraded (see `tco`): emit a landed real call so the effect happens
			// before the `finally`.
			Rvalue::TailCall(c, args) => {
				if self.tco {
					format!("new __TC({}, [{}])", self.atom(c), self.atoms(args))
				} else {
					format!("__land({}({}))", self.atom(c), self.atoms(args))
				}
			}
			// Only produced by the VM's `ir::optimize` (which the JS backend doesn't
			// run), so unreachable here; lower it as a plain direct call for safety.
			Rvalue::TailCallDirect(f, args) => self.call(&Callee::Function(*f), args)?,
			Rvalue::GetDictMethod(d, i) => format!("{}[{i}]", self.atom(d)),
			Rvalue::MakeDict(methods) => format!("[{}]", self.atoms(methods)),
			Rvalue::MakeClosure(fid, caps) => {
				format!("__mkclosure($f{}, [{}])", fid.0, self.atoms(caps))
			}
			Rvalue::MakeRecord(fields) => {
				let parts: Vec<String> = fields
					.iter()
					.map(|(n, a)| format!("{}: {}", js_str(n), self.atom(a)))
					.collect();
				format!("{{{}}}", parts.join(", "))
			}
			Rvalue::RecordUpdate { base, fields } => {
				let mut parts = vec![format!("...{}", self.atom(base))];
				for (n, a) in fields {
					parts.push(format!("{}: {}", js_str(n), self.atom(a)));
				}
				format!("{{{}}}", parts.join(", "))
			}
			Rvalue::GetField(recv, name, _) => format!("{}[{}]", self.atom(recv), js_str(name)),
			Rvalue::GetElement(recv, idx) => format!("{}.e[{idx}]", self.atom(recv)),
			Rvalue::MakeVariant {
				enum_name,
				tag,
				payload,
			} => {
				let name = self.variant_name(enum_name, *tag)?;
				format!(
					"new PVariant({}, {tag}, {}, [{}])",
					js_str(enum_name),
					js_str(name),
					self.atoms(payload)
				)
			}
			Rvalue::MakeVariantCtor { enum_name, tag } => {
				let name = self.variant_name(enum_name, *tag)?;
				let arity = self.variant_arity(enum_name, *tag)?;
				let args: Vec<String> = (0..arity).map(|i| format!("__a{i}")).collect();
				format!(
					"(({}) => new PVariant({}, {tag}, {}, [{}]))",
					args.join(", "),
					js_str(enum_name),
					js_str(name),
					args.join(", ")
				)
			}
			Rvalue::Interpolate(parts) => {
				let mut s = String::from("(\"\"");
				for p in parts {
					write!(s, " + {}", self.atom(p)).unwrap();
				}
				s.push(')');
				s
			}
			Rvalue::GetTag(a) => format!("{}.tag", self.atom(a)),
			Rvalue::GetPayload(a, i) => format!("{}.p[{i}]", self.atom(a)),
			Rvalue::MakeList(items) => {
				let parts: Vec<String> = items
					.iter()
					.map(|it| match it {
						ListItem::Elem(a) => self.atom(a),
						ListItem::Spread(a) => format!("...{}", self.atom(a)),
					})
					.collect();
				format!("[{}]", parts.join(", "))
			}
			Rvalue::MakeTuple(elems) => format!("new PTuple([{}])", self.atoms(elems)),
			Rvalue::GlobalRef(g) => format!("__gload({})", g.0),
			Rvalue::Builtin(tag) => format!("RT[{}]", js_str(tag)),
			Rvalue::Await(_) => return Err("js: Await is out of scope (sync backend only)".into()),
		})
	}

	fn call(&self, callee: &Callee, args: &[Atom]) -> Result<String, String> {
		Ok(match callee {
			// Direct calls into Pluma code are landed: the target's body may end in
			// a tail call and hand back a `__TC` bounce. Builtins are native JS and
			// never bounce (those that invoke a closure land it internally).
			Callee::Function(f) => {
				format!("__land($f{}(null{}))", f.0, prepend_args(self.atoms(args)))
			}
			Callee::Global(g) => format!("__land(__gload({})({}))", g.0, self.atoms(args)),
			Callee::Builtin(tag) => format!("RT[{}]({})", js_str(tag), self.atoms(args)),
		})
	}

	fn binop(&self, op: BinOp, a: &Atom, b: &Atom) -> String {
		let (x, y) = (self.atom(a), self.atom(b));
		match op {
			BinOp::AddInt => format!("({x} + {y})"),
			BinOp::SubInt => format!("({x} - {y})"),
			BinOp::MulInt => format!("({x} * {y})"),
			BinOp::DivInt => format!("RT[\"int-div\"]({x}, {y})"),
			BinOp::RemInt => format!("({x} % {y})"),
			BinOp::AddFloat => format!("new PFloat({x}.v + {y}.v)"),
			BinOp::SubFloat => format!("new PFloat({x}.v - {y}.v)"),
			BinOp::MulFloat => format!("new PFloat({x}.v * {y}.v)"),
			BinOp::DivFloat => format!("new PFloat({x}.v / {y}.v)"),
			BinOp::RemFloat => format!("new PFloat({x}.v % {y}.v)"),
			BinOp::Concat => format!("({x} + {y})"),
			BinOp::And => format!("({x} && {y})"),
			BinOp::Or => format!("({x} || {y})"),
			BinOp::Eq => format!("__eq({x}, {y})"),
			BinOp::Ne => format!("(!__eq({x}, {y}))"),
			BinOp::LtI64 => format!("({x} < {y})"),
			BinOp::LeI64 => format!("({x} <= {y})"),
			BinOp::GtI64 => format!("({x} > {y})"),
			BinOp::GeI64 => format!("({x} >= {y})"),
			BinOp::LtF64 => format!("({x}.v < {y}.v)"),
			BinOp::LeF64 => format!("({x}.v <= {y}.v)"),
			BinOp::GtF64 => format!("({x}.v > {y}.v)"),
			BinOp::GeF64 => format!("({x}.v >= {y}.v)"),
		}
	}

	fn atom(&self, a: &Atom) -> String {
		match a {
			Atom::Var(v) => var(*v),
			Atom::Const(c) => js_const(c),
		}
	}

	fn atoms(&self, args: &[Atom]) -> String {
		args
			.iter()
			.map(|a| self.atom(a))
			.collect::<Vec<_>>()
			.join(", ")
	}

	fn fresh(&mut self, prefix: &str) -> String {
		let n = self.tmp;
		self.tmp += 1;
		format!("{prefix}{n}")
	}

	// ---- globals + enum bindings -----------------------------------------

	fn emit_globals(&self, out: &mut String) -> Result<(), String> {
		// Push into the runtime's `__GINIT` array (mutated in place rather than
		// reassigned, so a `const` declaration in the preamble is fine).
		out.push_str("\n// ===== globals =====\n__GINIT.push(\n");
		for g in &self.program.globals {
			match g {
				GlobalInit::Thunk(fid) => {
					writeln!(out, "\tnew Thunk(() => $f{}(null)),", fid.0).unwrap();
				}
				GlobalInit::PreEvaluated(p) => {
					writeln!(out, "\t{},", self.pre_eval(p)?).unwrap();
				}
			}
		}
		out.push_str(");\n");
		Ok(())
	}

	fn pre_eval(&self, p: &PreEval) -> Result<String, String> {
		Ok(match p {
			PreEval::Builtin(tag) => format!("RT[{}]", js_str(tag)),
			PreEval::Const(c) => js_const(c),
			PreEval::MethodDict(items) => {
				let parts: Result<Vec<String>, String> = items.iter().map(|i| self.pre_eval(i)).collect();
				format!("[{}]", parts?.join(", "))
			}
		})
	}

	/// Bind the program's `option`/`result` enums so native builtins that return
	/// them (`dict.lookup`, the io-result wrappers, …) build the right variants.
	fn emit_enum_bindings(&self, out: &mut String) {
		let find = |a: &str, b: &str| {
			self.enums.iter().find_map(|(name, vs)| {
				if vs.len() == 2 && vs[0].0 == a && vs[1].0 == b {
					Some(name.clone())
				} else {
					None
				}
			})
		};
		let option = find("some", "none");
		let result = find("ok", "err");
		// The `ordering` enum (3 variants: lt/eq/gt) backs the compare builtins.
		let ordering = self.enums.iter().find_map(|(name, vs)| {
			if vs.len() == 3 && vs[0].0 == "lt" && vs[1].0 == "eq" && vs[2].0 == "gt" {
				Some(name.clone())
			} else {
				None
			}
		});
		let q = |o: Option<String>| o.map(|s| js_str(&s)).unwrap_or_else(|| "null".into());
		writeln!(
			out,
			"__bindEnums({}, {}, {});",
			q(option),
			q(result),
			q(ordering),
		)
		.unwrap();
	}
}

// ---- free helpers --------------------------------------------------------

/// `_<n>` — the JS local for a `VarId`.
fn var(v: VarId) -> String {
	format!("_{}", v.0)
}

/// `let _<n>` — a fresh `let` declaration for a capture binding.
fn decl_var(v: VarId) -> String {
	format!("let _{}", v.0)
}

fn prepend_args(rest: String) -> String {
	if rest.is_empty() {
		String::new()
	} else {
		format!(", {rest}")
	}
}

/// A JS expression for an inline constant.
fn js_const(c: &Const) -> String {
	match c {
		Const::Unit => "NOTHING".to_string(),
		Const::Bool(b) => b.to_string(),
		Const::Int(n) => n.to_string(),
		Const::Float(f) => format!("new PFloat({})", js_float(*f)),
		Const::Str(s) => js_str(s),
		Const::Bytes(b) => js_bytes(b),
		Const::Duration(n) => format!("new PDuration({n}n)"),
	}
}

/// A JS numeric literal for an f64 (handles the non-finite cases).
fn js_float(f: f64) -> String {
	if f.is_nan() {
		"NaN".to_string()
	} else if f == f64::INFINITY {
		"Infinity".to_string()
	} else if f == f64::NEG_INFINITY {
		"-Infinity".to_string()
	} else {
		// Rust's `{:?}` for f64 round-trips and is valid JS numeric syntax.
		let s = format!("{f:?}");
		s
	}
}

/// A double-quoted, escaped JS string literal.
fn js_str(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for ch in s.chars() {
		match ch {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => {
				write!(out, "\\u{:04x}", c as u32).unwrap();
			}
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// A `Uint8Array` literal for a byte slice.
fn js_bytes(b: &[u8]) -> String {
	if b.is_empty() {
		"new Uint8Array(0)".to_string()
	} else {
		let nums: Vec<String> = b.iter().map(|x| x.to_string()).collect();
		format!("Uint8Array.of({})", nums.join(", "))
	}
}

/// Collect every `let`-bound and pattern-bound `VarId` in a block (recursively),
/// so the function can hoist a `let` for each — mirroring the VM's flat frame
/// slots, which keeps a var written in one branch/arm live after the join.
fn collect_bound(block: &Block, out: &mut Vec<u32>) {
	for stmt in &block.0 {
		match &stmt.kind {
			StmtKind::Let(v, _) => out.push(v.0),
			StmtKind::If(_, t, e) => {
				collect_bound(t, out);
				collect_bound(e, out);
			}
			StmtKind::Switch { arms, default, .. } => {
				for (_, b) in arms {
					collect_bound(b, out);
				}
				collect_bound(default, out);
			}
			StmtKind::Loop(b) => collect_bound(b, out),
			StmtKind::Match { arms, .. } => {
				for arm in arms {
					collect_pattern_bound(&arm.pattern, out);
					collect_bound(&arm.body, out);
				}
			}
			_ => {}
		}
	}
}

fn collect_pattern_bound(pat: &Pattern, out: &mut Vec<u32>) {
	match pat {
		Pattern::Bind(v) => out.push(v.0),
		Pattern::Variant { fields, .. } | Pattern::Tuple(fields) => {
			for f in fields {
				collect_pattern_bound(f, out);
			}
		}
		Pattern::List { items, rest } => {
			for i in items {
				collect_pattern_bound(i, out);
			}
			if let Some(ListRest::Bind(v)) = rest {
				out.push(v.0);
			}
		}
		Pattern::Record { fields, rest, .. } => {
			for (_, p) in fields {
				collect_pattern_bound(p, out);
			}
			if let RecordRest::Bind(v) = rest {
				out.push(v.0);
			}
		}
		Pattern::Wildcard | Pattern::Literal(_) => {}
	}
}

fn block_has_defer(block: &Block) -> bool {
	block.0.iter().any(|s| match &s.kind {
		StmtKind::PushDefer(_) => true,
		StmtKind::If(_, t, e) => block_has_defer(t) || block_has_defer(e),
		StmtKind::Switch { arms, default, .. } => {
			arms.iter().any(|(_, b)| block_has_defer(b)) || block_has_defer(default)
		}
		StmtKind::Loop(b) => block_has_defer(b),
		StmtKind::Match { arms, .. } => arms.iter().any(|a| block_has_defer(&a.body)),
		_ => false,
	})
}
