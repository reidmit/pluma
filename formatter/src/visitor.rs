use compiler::ast::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::doc::*;

pub(crate) struct Formatter<'a> {
	comments: &'a HashMap<usize, String>,
	consumed: RefCell<HashSet<usize>>,
	// True while printing an `if`/`while` subject spine, where a `{` opens the
	// body: a record literal reached at this level must be parenthesized to
	// round-trip. Set by `format_subject`, lifted on descent into any delimiter
	// or keyword-led construct (see `fmt_prec`).
	restrict_brace: Cell<bool>,
}

impl<'a> Formatter<'a> {
	pub fn new(comments: &'a HashMap<usize, String>) -> Self {
		Self {
			comments,
			consumed: RefCell::new(HashSet::new()),
			restrict_brace: Cell::new(false),
		}
	}

	pub fn format_module(&self, m: &ModuleNode) -> Doc {
		let mut parts: Vec<Doc> = Vec::new();

		// Leading comments before the first import/def get emitted first;
		// they belong to the module as a preamble.
		let first_line = m
			.uses
			.first()
			.map(|u| u.range.start.line)
			.or_else(|| m.body.first().map(|d| d.range.start.line))
			.unwrap_or(usize::MAX);
		parts.push(self.drain_leading(first_line));

		// Imports: sorted into canonical order (stdlib, then package, then
		// user; within each group by segment count, then alphabetically),
		// one per line, no blank between consecutive imports.
		let mut uses: Vec<&UseNode> = m.uses.iter().collect();
		uses.sort_by_key(|u| (use_group_rank(u), u.path.len(), u.module_name()));
		for (i, u) in uses.iter().enumerate() {
			if i > 0 {
				parts.push(hardline());
			}
			parts.push(self.format_use(u));
			parts.push(self.trailing_comment(u.range.end.line));
		}

		// Blank line between import block and def block.
		if !m.uses.is_empty() && !m.body.is_empty() {
			parts.push(hardline());
			parts.push(hardline());
		}

		// Definitions: one blank line between consecutive defs. Leading
		// comments for each def go right above it (with a blank line
		// separating from the prior def).
		for (i, def) in m.body.iter().enumerate() {
			if i > 0 {
				parts.push(hardline());
				parts.push(hardline());
			}
			parts.push(self.drain_leading(def.range.start.line));
			parts.push(self.format_definition(def));
			parts.push(self.trailing_comment(def.range.end.line));
		}

		// Remaining comments at end of file (e.g. notes after the last def).
		let remaining_lines: Vec<usize> = self
			.comments
			.keys()
			.filter(|&&l| !self.consumed.borrow().contains(&l))
			.copied()
			.collect();
		if !remaining_lines.is_empty() {
			parts.push(hardline());
			parts.push(hardline());
			parts.push(self.drain_leading(usize::MAX));
		}

		concat(parts)
	}

	// --- comments -----------------------------------------------------

	// Emit any comments on lines strictly less than `target_line` that
	// haven't been consumed yet. Each comment is followed by a hardline, so
	// after this Doc the cursor is at the start of a fresh line. Source
	// blank-line gaps between consecutive comments are preserved, and a
	// trailing blank line is emitted if there was one between the last
	// comment and `target_line`.
	fn drain_leading(&self, target_line: usize) -> Doc {
		let mut lines: Vec<usize> = self
			.comments
			.keys()
			.filter(|&&l| l < target_line && !self.consumed.borrow().contains(&l))
			.copied()
			.collect();
		lines.sort();

		if lines.is_empty() {
			return nil();
		}

		let mut parts: Vec<Doc> = Vec::new();
		let mut prev: Option<usize> = None;
		for line in lines {
			if let Some(p) = prev {
				if line > p + 1 {
					parts.push(hardline());
				}
			}
			let body = self.comments.get(&line).unwrap();
			parts.push(text(format!("#{}", body)));
			parts.push(hardline());
			self.consumed.borrow_mut().insert(line);
			prev = Some(line);
		}

		// Preserve a blank line between the comment block and the next
		// code, if the source had one.
		if let Some(last) = prev {
			if target_line > last + 1 {
				parts.push(hardline());
			}
		}

		concat(parts)
	}

	// A comment sharing a node's last line sits after that node's final token
	// — a `#` comment runs to end of line — so it's a trailing comment for the
	// node. Emit it inline as ` # ...` and mark it consumed, so `drain_leading`
	// won't later hoist it onto its own line above the following item (which
	// would also detach it from the value it annotates). Returns nil when no
	// unconsumed comment lands on `end_line`.
	fn trailing_comment(&self, end_line: usize) -> Doc {
		if self.consumed.borrow().contains(&end_line) {
			return nil();
		}
		match self.comments.get(&end_line) {
			Some(body) => {
				self.consumed.borrow_mut().insert(end_line);
				text(format!(" #{}", body))
			}
			None => nil(),
		}
	}

	// --- use ----------------------------------------------------------

	fn format_use(&self, u: &UseNode) -> Doc {
		let mut parts: Vec<Doc> = vec![text("use "), text(u.module_name())];
		if let Some(alias) = &u.alias {
			parts.push(text(" as "));
			parts.push(text(alias.name.clone()));
		}
		concat(parts)
	}

	// --- definitions --------------------------------------------------

	fn format_definition(&self, def: &DefinitionNode) -> Doc {
		let inner = match &def.kind {
			// `def NAME = fun { ... }` always lays the body out multi-line,
			// even when a one-expression body would otherwise fit inline.
			// Defs read as "this is a thing" and benefit from a stable
			// vertical shape. This applies to top-level defs and to
			// instance methods inside `implement { ... }` alike.
			DefinitionKind::Expr(expr) => {
				let value_doc = match &expr.kind {
					ExprKind::Fun(fun) => self.format_fun_block(fun),
					_ => self.format_expr(expr),
				};
				// `def NAME [:: TYPE] [where (...)] = value`. The annotation
				// and `where` clause are the def's contract — they must be
				// re-emitted or the formatter would silently drop the program's
				// declared type.
				// `remote def` — the endpoint modifier sits between visibility
				// and `def`, so it's emitted here (inner) and `public` is
				// prepended below: `public remote def`. Dropping it would
				// silently demote an RPC endpoint to a plain def.
				let mut parts: Vec<Doc> = Vec::new();
				if def.is_remote {
					parts.push(text("remote "));
				}
				parts.push(text("def "));
				parts.push(text(def.name.name.clone()));
				if let Some(ty) = &def.type_annotation {
					parts.push(text(" :: "));
					parts.push(self.format_type_expr(ty));
				}
				if !def.where_clause.is_empty() {
					parts.push(self.format_where_clause(&def.where_clause));
				}
				parts.push(text(" = "));
				parts.push(value_doc);
				concat(parts)
			}
			DefinitionKind::Alias(ty) => concat(vec![
				text("alias "),
				text(def.name.name.clone()),
				text(" "),
				self.format_type_expr(ty),
			]),
			DefinitionKind::Enum(en) => self.format_enum(&def.name.name, en),
			DefinitionKind::Trait(tr) => self.format_trait(&def.name.name, tr),
			DefinitionKind::Instance(inst) => self.format_instance(inst),
		};
		// Re-emit the leading visibility keyword. Dropping it would silently
		// change a public/opaque def back to private, so this is part of
		// lossless round-tripping.
		match def.visibility {
			Visibility::Private => inner,
			Visibility::Opaque => concat(vec![text("opaque "), inner]),
			Visibility::Public => concat(vec![text("public "), inner]),
		}
	}

	// Like `format_fun`, but always emits the body multi-line (even for a
	// single-expression body that would otherwise fit on one line).
	fn format_fun_block(&self, fun: &FunNode) -> Doc {
		let mut head: Vec<Doc> = vec![text("fun")];
		for p in &fun.params {
			head.push(text(" "));
			head.push(text(p.ident.name.clone()));
		}
		head.push(text(" {"));

		if fun.body.is_empty() {
			return concat(vec![concat(head), text("}")]);
		}

		concat(vec![
			concat(head),
			nest(self.format_statements(&fun.body)),
			hardline(),
			text("}"),
		])
	}

	fn format_enum(&self, name: &str, en: &EnumNode) -> Doc {
		let mut header: Vec<Doc> = vec![text("enum "), text(name.to_string())];
		for p in &en.params {
			header.push(text(" "));
			header.push(text(p.name.clone()));
		}
		header.push(text(" {"));

		let mut body: Vec<Doc> = Vec::new();
		let mut prev_line: Option<usize> = None;
		for v in &en.variants {
			body.push(hardline());
			if let Some(p) = prev_line {
				let next_line = self.first_unconsumed_line(v.range.start.line);
				if next_line > p + 1 {
					body.push(hardline());
				}
			}
			body.push(self.drain_leading(v.range.start.line));
			body.push(self.format_enum_variant(v));
			body.push(self.trailing_comment(v.range.end.line));
			prev_line = Some(v.range.end.line);
		}

		concat(vec![
			concat(header),
			nest(concat(body)),
			hardline(),
			text("}"),
		])
	}

	fn format_enum_variant(&self, v: &EnumVariantNode) -> Doc {
		let mut parts: Vec<Doc> = vec![text(v.name.name.clone())];
		if let Some(params) = &v.params {
			for p in params {
				parts.push(text(" "));
				parts.push(self.format_type_expr(p));
			}
		}
		concat(parts)
	}

	fn format_trait(&self, name: &str, tr: &TraitNode) -> Doc {
		let header = concat(vec![
			text("trait "),
			text(name.to_string()),
			text(" "),
			text(tr.param.name.clone()),
			text(" {"),
		]);

		let mut body: Vec<Doc> = Vec::new();
		let mut prev_line: Option<usize> = None;
		for m in &tr.methods {
			body.push(hardline());
			if let Some(p) = prev_line {
				let next_line = self.first_unconsumed_line(m.range.start.line);
				if next_line > p + 1 {
					body.push(hardline());
				}
			}
			body.push(self.drain_leading(m.range.start.line));
			body.push(self.format_trait_method(m));
			body.push(self.trailing_comment(m.range.end.line));
			prev_line = Some(m.range.end.line);
		}

		concat(vec![header, nest(concat(body)), hardline(), text("}")])
	}

	fn format_trait_method(&self, m: &TraitMethodNode) -> Doc {
		let mut parts: Vec<Doc> = vec![
			text(m.name.name.clone()),
			text(" :: "),
			self.format_type_expr(&m.signature),
		];
		if let Some(default) = &m.default {
			parts.push(hardline());
			parts.push(text("def "));
			parts.push(text(m.name.name.clone()));
			parts.push(text(" = "));
			parts.push(self.format_expr(default));
		}
		concat(parts)
	}

	fn format_instance(&self, inst: &InstanceNode) -> Doc {
		let mut header: Vec<Doc> = vec![
			text("implement "),
			text(inst.trait_name.name.clone()),
			text(" "),
			self.format_type_expr(&inst.head),
		];
		if !inst.where_clause.is_empty() {
			header.push(self.format_where_clause(&inst.where_clause));
		}
		header.push(text(" {"));

		let mut body: Vec<Doc> = Vec::new();
		let mut prev_line: Option<usize> = None;
		for m in &inst.methods {
			body.push(hardline());
			if let Some(p) = prev_line {
				let next_line = self.first_unconsumed_line(m.range.start.line);
				if next_line > p + 1 {
					body.push(hardline());
				}
			}
			body.push(self.drain_leading(m.range.start.line));
			body.push(self.format_definition(m));
			body.push(self.trailing_comment(m.range.end.line));
			prev_line = Some(m.range.end.line);
		}

		concat(vec![
			concat(header),
			nest(concat(body)),
			hardline(),
			text("}"),
		])
	}

	// `where (trait param, trait param, ...)` — shared by instance heads
	// (`implement TRAIT TYPE where (...)`) and top-level def signatures
	// (`def name :: TYPE where (...) = ...`). Emits a leading space.
	fn format_where_clause(&self, constraints: &[InstanceConstraintNode]) -> Doc {
		let docs: Vec<Doc> = constraints
			.iter()
			.map(|c| {
				concat(vec![
					text(c.trait_name.name.clone()),
					text(" "),
					text(c.param.name.clone()),
				])
			})
			.collect();
		concat(vec![text(" where ("), join(text(", "), docs), text(")")])
	}

	// --- expressions --------------------------------------------------

	// Format an expression in a lowest-binding-power context: statement
	// position, or anywhere a child is delimited by its own brackets (list
	// elements, record/tuple fields, interpolation holes, if/when subjects and
	// bodies, let/try values). At `min_prec` 0 nothing is ever wrapped, so all
	// redundant parens are dropped. Precedence-sensitive positions (operator
	// operands, call callee/args, access receivers) call `fmt` directly.
	fn format_expr(&self, e: &ExprNode) -> Doc {
		self.fmt(e, 0)
	}

	// Format `e` where the surrounding parser context requires an operand
	// binding at least as tightly as `min_prec`. We re-derive parentheses
	// purely from precedence rather than echoing source parens: a source
	// `Grouping` is transparent (recurse through it), and we re-insert parens
	// iff `e`'s top-level operator binds looser than the context needs. This
	// removes every redundant paren while preserving the parse — `min_prec`
	// mirrors the `min_bp` the parser would use at this position (see
	// `Operator::infix_binding_power` and the call/access arms below).
	fn fmt(&self, e: &ExprNode, min_prec: u8) -> Doc {
		self.fmt_prec(e, min_prec, false)
	}

	// Like `fmt`, but `e` sits in statement-tail position: a `fun` reached
	// along its right spine — the final argument of a call, a `let`/`try`
	// value, or `e` itself — always lays out multi-line, even when its body
	// would fit on one line. This is the same stable vertical shape
	// `def NAME = fun { ... }` gets (see `format_fun_block`), so a trailing
	// lambda like `t.case "x" fun { assert.is-ok r }` breaks across lines.
	fn fmt_tail(&self, e: &ExprNode, min_prec: u8) -> Doc {
		self.fmt_prec(e, min_prec, true)
	}

	fn fmt_prec(&self, e: &ExprNode, min_prec: u8, tail: bool) -> Doc {
		if let ExprKind::Grouping(inner) = &e.kind {
			// Transparent: a source grouping carries no restriction of its own.
			return self.fmt_prec(inner, min_prec, tail);
		}
		// In an `if`/`while` subject, a bare record literal would have its `{`
		// read as the body, so wrap it (`f ({ x: 1 })`). Inside the parens the
		// restriction is lifted, like any delimiter.
		let restrict = self.restrict_brace.get();
		let brace_guard =
			restrict && matches!(e.kind, ExprKind::Record(..) | ExprKind::RecordUpdate { .. });
		if brace_guard || expr_prec(e) < min_prec {
			let saved = self.restrict_brace.replace(false);
			let doc = self.render_expr(e, tail);
			self.restrict_brace.set(saved);
			return concat(vec![text("("), doc, text(")")]);
		}
		// No parens here. The restriction only follows the brace-sensitive spine
		// — nodes whose children print bare (calls, operators, accesses,
		// `defer`/`try`/`let` operands). Everything else wraps its children in a
		// delimiter or is keyword-led, so lift the restriction for them.
		if restrict && !is_brace_spine(e) {
			let saved = self.restrict_brace.replace(false);
			let doc = self.render_expr(e, tail);
			self.restrict_brace.set(saved);
			doc
		} else {
			self.render_expr(e, tail)
		}
	}

	// Render `e`'s own syntax, recursing into children at the binding power
	// each child position demands. Never wraps `e` itself in parens — that's
	// `fmt`'s job, based on the caller's context. `Grouping` is unreachable
	// here (peeled in `fmt`); handled defensively as a passthrough.
	fn render_expr(&self, e: &ExprNode, tail: bool) -> Doc {
		use ExprKind::*;
		match &e.kind {
			Literal(lit) => self.format_literal(lit),
			Identifier(ident) => text(ident.name.clone()),
			EmptyTuple => text("()"),
			Grouping(inner) => self.fmt(inner, 0),
			BinaryOperation { op, left, right } => {
				let p = op_prec(&op.kind);
				// Right-associative ops want their *left* child to bind one
				// step tighter (so `(a ?? b) ?? c` keeps parens); left-assoc
				// ops want their *right* child tighter (so `a - (b - c)` does).
				let (left_mp, right_mp) = if is_right_assoc(&op.kind) {
					(p + 1, p)
				} else {
					(p, p + 1)
				};
				concat(vec![
					self.fmt(left, left_mp),
					text(" "),
					text(format!("{}", op.kind)),
					text(" "),
					self.fmt(right, right_mp),
				])
			}
			UnaryOperation { op, right } => concat(vec![
				text(format!("{}", op)),
				self.fmt(right, prefix_prec(op)),
			]),
			ElementAccess { receiver, index } => concat(vec![
				self.fmt(receiver, 100),
				text("."),
				text(index.to_string()),
			]),
			FieldAccess { receiver, field } => concat(vec![
				self.fmt(receiver, 100),
				text("."),
				text(field.name.clone()),
			]),
			NamespaceAccess(path) => {
				// The formatter runs on parser output, which never produces
				// NamespaceAccess (the analyzer creates it). Handle it for
				// completeness so analyzed ASTs can also round-trip.
				let mut parts: Vec<Doc> = Vec::new();
				for (i, segment) in path.iter().enumerate() {
					if i > 0 {
						parts.push(text("."));
					}
					parts.push(text(segment.name.clone()));
				}
				concat(parts)
			}
			Fun(fun) => {
				if tail {
					self.format_fun_block(fun)
				} else {
					self.format_fun(fun)
				}
			}
			Call(call) => self.format_call(call, tail),
			Let(l) => self.format_let(l, tail),
			Defer(inner) => concat(vec![text("defer "), self.fmt_prec(inner, 0, tail)]),
			Try(t) => self.format_try(t, tail),
			Tuple(items) => self.format_tuple(items),
			List(items) => self.format_list(items),
			Record(fields) => self.format_record(fields),
			RecordUpdate { base, fields } => self.format_record_update(base, fields),
			Interpolation(parts) => self.format_interpolation(parts),
			Regex(r) => self.format_regex_literal(r),
			If(i) => self.format_if(i),
			When(w) => self.format_when(w),
			While(w) => self.format_while(w),
			Scope(s) => self.format_scope(s),
			Builtin(tag) => concat(vec![text("built-in \""), text(tag.clone()), text("\"")]),
		}
	}

	fn format_literal(&self, lit: &LiteralNode) -> Doc {
		match &lit.kind {
			LiteralKind::Bool(true) => text("true"),
			LiteralKind::Bool(false) => text("false"),
			LiteralKind::IntDecimal(n) => text(n.to_string()),
			LiteralKind::IntHex(n) => text(format!("0x{:x}", n)),
			LiteralKind::IntOctal(n) => text(format!("0o{:o}", n)),
			LiteralKind::IntBinary(n) => text(format!("0b{:b}", n)),
			LiteralKind::FloatDecimal(f) => text(format_float(*f)),
			LiteralKind::Duration(n) => text(format_duration(*n)),
			LiteralKind::String(s) => text(format!("\"{}\"", escape_string(s))),
			LiteralKind::Bytes(b) => text(format!("'{}'", escape_bytes(b))),
		}
	}

	fn format_fun(&self, fun: &FunNode) -> Doc {
		let mut head: Vec<Doc> = vec![text("fun")];
		for p in &fun.params {
			head.push(text(" "));
			head.push(text(p.ident.name.clone()));
		}
		head.push(text(" {"));

		if fun.body.is_empty() {
			return concat(vec![concat(head), text("}")]);
		}

		// Single-expression body with no associated leading comments lays
		// out flat when it fits; otherwise it breaks.
		if fun.body.len() == 1 && !self.has_leading_comments(fun.body[0].range.start.line) {
			let only = &fun.body[0];
			return group(concat(vec![
				concat(head),
				nest(concat(vec![line(), self.fmt_tail(only, 0)])),
				line(),
				text("}"),
			]));
		}

		let body_parts = self.format_statements(&fun.body);
		concat(vec![concat(head), nest(body_parts), hardline(), text("}")])
	}

	// Lay out a sequence of statements (fun body, if/when/while body), each
	// on its own line. Preserves source blank lines between statements and
	// attaches leading comments. Each call returns a Doc that begins with
	// a hardline (so the caller can compose it after a `{` directly).
	fn format_statements(&self, exprs: &[ExprNode]) -> Doc {
		let mut parts: Vec<Doc> = Vec::new();
		let mut prev_end: Option<usize> = None;
		for expr in exprs {
			parts.push(hardline());
			if let Some(p) = prev_end {
				// Emit a blank line if the source had one between the
				// previous statement and the next thing that gets emitted
				// (which may be a leading comment, not the statement itself).
				let next_line = self.first_unconsumed_line(expr.range.start.line);
				if next_line > p + 1 {
					parts.push(hardline());
				}
			}
			parts.push(self.drain_leading(expr.range.start.line));
			parts.push(self.fmt_tail(expr, 0));
			parts.push(self.trailing_comment(expr.range.end.line));
			prev_end = Some(expr.range.end.line);
		}
		concat(parts)
	}

	fn has_leading_comments(&self, target_line: usize) -> bool {
		self
			.comments
			.keys()
			.any(|&l| l < target_line && !self.consumed.borrow().contains(&l))
	}

	// Earliest source line strictly less than `target_line` that contains
	// either an unconsumed comment or, if no such comment exists,
	// `target_line` itself. Used by callers that want to decide whether to
	// emit a blank line between `prev_end_line` and whatever the next
	// emitted item is.
	fn first_unconsumed_line(&self, target_line: usize) -> usize {
		self
			.comments
			.keys()
			.filter(|&&l| l < target_line && !self.consumed.borrow().contains(&l))
			.copied()
			.min()
			.unwrap_or(target_line)
	}

	fn format_call(&self, call: &CallNode, tail: bool) -> Doc {
		// Calls are whitespace-separated callee + args. Pluma application is
		// newline-terminated — a call ends at the first newline that isn't
		// inside an open bracket — so we must NOT break a call across lines:
		// args are always joined by a single space. An argument may still be
		// internally multi-line (a `fun` block, a multi-line list/record),
		// but those line breaks live inside the argument's own brackets.
		// The callee binds at function-call precedence (90); each argument is
		// parsed at 91 (call's right binding power), so an argument keeps its
		// parens iff its top operator binds looser than 91 — i.e. anything but
		// a field/element access or a self-delimiting primary. A `fun`/`if`/
		// `when`/`while` literal is such a primary, so its wrapping parens drop
		// (`list.map xs (fun x { ... })` → `list.map xs fun x { ... }`); a
		// nested call or operator expression keeps them (`f (g x)`, `f (a + b)`).
		let mut parts: Vec<Doc> = vec![self.fmt(&call.callee, 90)];
		let last = call.args.len().wrapping_sub(1);
		for (i, arg) in call.args.iter().enumerate() {
			parts.push(text(" "));
			// Only the final argument continues the statement's tail: a `fun`
			// there ends the line, so it breaks (`t.case "x" fun { ... }`).
			// Earlier args are interior, so they stay inline when they fit.
			if tail && i == last {
				parts.push(self.fmt_tail(arg, 91));
			} else {
				parts.push(self.fmt(arg, 91));
			}
		}
		concat(parts)
	}

	fn format_let(&self, l: &LetNode, tail: bool) -> Doc {
		let mut parts: Vec<Doc> = vec![text("let "), self.format_pattern(&l.pattern)];
		if let Some(ty) = &l.type_annotation {
			parts.push(text(" :: "));
			parts.push(self.format_type_expr(ty));
		}
		parts.push(text(" = "));
		// The value ends the binding's line, so a `fun` there breaks just
		// like `def NAME = fun { ... }`.
		parts.push(self.fmt_prec(&l.value, 0, tail));
		concat(parts)
	}

	// `try` mirrors `let`'s shape: `try Pattern = Expr`. The continuation
	// (`rest`) is rendered as inline siblings — at the source level a try
	// has no braces around what follows, just subsequent expressions in
	// the enclosing block.
	fn format_try(&self, t: &TryNode, tail: bool) -> Doc {
		let mut parts: Vec<Doc> = vec![
			text("try "),
			self.format_pattern(&t.pattern),
			text(" = "),
			self.fmt_prec(&t.value, 0, tail),
			// The binding's value ends this line, so a comment on it is trailing.
			// Unlike a plain `let`, a `try`'s binding line isn't its own statement
			// in `format_statements` (the continuation is folded into `rest`), so
			// the trailing comment must be picked up here or it would be hoisted.
			self.trailing_comment(t.value.range.end.line),
		];
		// The continuation expressions are siblings in the enclosing block,
		// so each is itself a statement and carries the same tail position.
		for e in &t.rest {
			parts.push(hardline());
			parts.push(self.fmt_prec(e, 0, tail));
			parts.push(self.trailing_comment(e.range.end.line));
		}
		concat(parts)
	}

	fn format_tuple(&self, items: &[ExprNode]) -> Doc {
		if items.is_empty() {
			return text("()");
		}
		let docs: Vec<Doc> = items.iter().map(|e| self.format_expr(e)).collect();
		// Tuples stay inline when they fit, but wrap (one per line, trailing
		// comma) when too wide — same as lists and records.
		bracketed_collection("(", ")", docs)
	}

	fn format_list(&self, items: &[ListItem]) -> Doc {
		if items.is_empty() {
			return text("[]");
		}
		let docs: Vec<Doc> = items
			.iter()
			.map(|item| match item {
				ListItem::Item(e) => self.format_expr(e),
				ListItem::Spread(e) => concat(vec![text("..."), self.format_expr(e)]),
			})
			.collect();
		bracketed_collection("[", "]", docs)
	}

	fn format_record(&self, fields: &[(IdentifierNode, ExprNode)]) -> Doc {
		if fields.is_empty() {
			return text("{}");
		}
		let docs: Vec<Doc> = fields
			.iter()
			.map(|(name, value)| {
				// Field shorthand: render `{a: a}` as `{a}` when the value
				// is just an identifier with the same name.
				if let ExprKind::Identifier(ident) = &value.kind {
					if ident.name == name.name {
						return text(name.name.clone());
					}
				}
				concat(vec![
					text(name.name.clone()),
					text(": "),
					self.format_expr(value),
				])
			})
			.collect();
		bracketed_collection("{", "}", docs)
	}

	fn format_record_update(&self, base: &ExprNode, fields: &[(IdentifierNode, ExprNode)]) -> Doc {
		let mut docs: Vec<Doc> = Vec::with_capacity(fields.len() + 1);
		docs.push(concat(vec![text("..."), self.format_expr(base)]));
		for (name, value) in fields {
			// Field shorthand: render `{...r, a: a}` as `{...r, a}` when the
			// value is just an identifier with the same name.
			if let ExprKind::Identifier(ident) = &value.kind {
				if ident.name == name.name {
					docs.push(text(name.name.clone()));
					continue;
				}
			}
			docs.push(concat(vec![
				text(name.name.clone()),
				text(": "),
				self.format_expr(value),
			]));
		}
		bracketed_collection("{", "}", docs)
	}

	fn format_interpolation(&self, parts: &[ExprNode]) -> Doc {
		// Parts alternate: string literal, expression, string literal, ...
		let mut buf = String::from("\"");
		for (i, part) in parts.iter().enumerate() {
			if i % 2 == 0 {
				// String literal segment — already de-escaped in the AST.
				if let ExprKind::Literal(LiteralNode {
					kind: LiteralKind::String(s),
					..
				}) = &part.kind
				{
					buf.push_str(&escape_string(s));
				}
			} else {
				buf.push_str("$(");
				// Render the inner expression as a single line. The exact
				// width doesn't matter as long as it's wide enough to fit
				// any reasonable expression — usize::MAX would overflow when
				// cast to a signed budget, so use a large finite value.
				let inner = render(&self.format_expr(part), 1_000_000);
				buf.push_str(&inner);
				buf.push(')');
			}
		}
		buf.push('"');
		text(buf)
	}

	fn format_regex_literal(&self, r: &RegexNode) -> Doc {
		concat(vec![text("`"), self.format_regex(r), text("`")])
	}

	fn format_regex(&self, r: &RegexNode) -> Doc {
		use RegexKind::*;
		match &r.kind {
			Literal(s) => text(format!("\"{}\"", escape_string(s))),
			CharacterClass(name) => text(name.clone()),
			Anchor(a) => text(
				match a {
					RegexAnchor::Start => "^",
					RegexAnchor::End => "$",
					RegexAnchor::Boundary => "%",
				}
				.to_string(),
			),
			Sequence(parts) => {
				let docs: Vec<Doc> = parts.iter().map(|p| self.format_regex(p)).collect();
				join(text(" "), docs)
			}
			Alternation(parts) => {
				let docs: Vec<Doc> = parts.iter().map(|p| self.format_regex(p)).collect();
				join(text(" | "), docs)
			}
			Grouping(inner) => concat(vec![text("("), self.format_regex(inner), text(")")]),
			ZeroOrMore(inner) => concat(vec![self.format_regex(inner), text("*")]),
			OneOrMore(inner) => concat(vec![self.format_regex(inner), text("+")]),
			OneOrZero(inner) => concat(vec![self.format_regex(inner), text("?")]),
			ExactCount(inner, n) => concat(vec![self.format_regex(inner), text(format!("{{{}}}", n))]),
			AtLeastCount(inner, n) => concat(vec![self.format_regex(inner), text(format!("{{{},}}", n))]),
			AtMostCount(inner, n) => concat(vec![self.format_regex(inner), text(format!("{{,{}}}", n))]),
			RangeCount(inner, min, max) => concat(vec![
				self.format_regex(inner),
				text(format!("{{{},{}}}", min, max)),
			]),
			NamedCapture(name, inner) => concat(vec![
				text("<"),
				text(name.clone()),
				text(": "),
				self.format_regex(inner),
				text(">"),
			]),
		}
	}

	// Print an `if`/`while` subject, parenthesizing it when its bare form would
	// expose a `{` the brace-restricted parser would read as the body.
	fn format_subject(&self, e: &ExprNode) -> Doc {
		let saved = self.restrict_brace.replace(true);
		let doc = self.format_expr(e);
		self.restrict_brace.set(saved);
		doc
	}

	fn format_if(&self, i: &IfNode) -> Doc {
		// `if` always lays out multi-line, mirroring `when`:
		//
		//     if SUBJECT is PATTERN {
		//         body
		//     } else {
		//         else_body
		//     }
		//
		// `} else {` sits on the same line as the body's closing brace. The
		// `is PATTERN` is elided for the canonical boolean case (`is true`).
		let mut parts: Vec<Doc> = vec![text("if "), self.format_subject(&i.subject)];
		if !pattern_is_true(&i.pattern) {
			parts.push(text(" is "));
			parts.push(self.format_pattern(&i.pattern));
		}
		parts.push(text(" {"));
		parts.push(nest(self.format_statements(&i.body)));
		parts.push(hardline());
		match &i.else_body {
			// `else if ...`: the chained `if` is the sole else expression. Render
			// it inline as `} else if ...` (the nested call emits its own closing
			// `}`), mirroring the parser so chains stay flat instead of nesting.
			Some(else_body) => {
				if let [
					ExprNode {
						kind: ExprKind::If(inner),
						..
					},
				] = else_body.as_slice()
				{
					parts.push(text("} else "));
					parts.push(self.format_if(inner));
				} else {
					parts.push(text("} else {"));
					parts.push(nest(self.format_statements(else_body)));
					parts.push(hardline());
					parts.push(text("}"));
				}
			}
			None => parts.push(text("}")),
		}
		concat(parts)
	}

	fn format_when(&self, w: &WhenNode) -> Doc {
		// `when` always lays out multi-line, with each arm's `is PAT {` (or
		// `else {`) on the line of the previous arm's closing brace:
		//
		//     when SUBJECT is PAT1 {
		//         body1
		//     } is PAT2 {
		//         body2
		//     } else {
		//         else_body
		//     }
		//
		// A trailing Underscore pattern is rendered as `else` (it's how the
		// parser desugars `else`). Underscore in a non-final position keeps
		// `is _` so semantics aren't lost.
		let mut parts: Vec<Doc> = vec![text("when "), self.format_expr(&w.subject)];
		let last_index = w.cases.len().saturating_sub(1);
		for (i, case) in w.cases.iter().enumerate() {
			let is_else = i == last_index && matches!(case.pattern.kind, PatternKind::Underscore);

			if i == 0 {
				parts.push(text(" "));
			} else {
				parts.push(text("} "));
			}

			if is_else {
				parts.push(text("else {"));
			} else {
				parts.push(text("is "));
				parts.push(self.format_pattern(&case.pattern));
				parts.push(text(" {"));
			}

			parts.push(nest(self.format_statements(&case.body)));
			parts.push(hardline());
		}
		parts.push(text("}"));
		concat(parts)
	}

	fn format_while(&self, w: &WhileNode) -> Doc {
		let mut parts: Vec<Doc> = vec![text("while "), self.format_subject(&w.subject)];
		if !pattern_is_true(&w.pattern) {
			parts.push(text(" is "));
			parts.push(self.format_pattern(&w.pattern));
		}
		parts.push(text(" "));
		parts.push(self.format_block(&w.body));
		concat(parts)
	}

	fn format_scope(&self, s: &ScopeNode) -> Doc {
		let mut parts: Vec<Doc> = Vec::new();
		if s.manual {
			parts.push(text("manual "));
		}
		parts.push(text("scope"));
		if let Some(handle) = &s.handle {
			parts.push(text(" as "));
			parts.push(text(handle.name.clone()));
		}
		parts.push(text(" "));
		parts.push(self.format_block(&s.body));
		concat(parts)
	}

	// A `{ ... }` block used for if/when/while branches. Lays flat when
	// the single-expression body fits; otherwise breaks.
	fn format_block(&self, body: &[ExprNode]) -> Doc {
		if body.is_empty() {
			return text("{}");
		}
		if body.len() == 1 && !self.has_leading_comments(body[0].range.start.line) {
			let only = &body[0];
			return group(concat(vec![
				text("{"),
				nest(concat(vec![line(), self.fmt_tail(only, 0)])),
				line(),
				text("}"),
			]));
		}

		concat(vec![
			text("{"),
			nest(self.format_statements(body)),
			hardline(),
			text("}"),
		])
	}

	// --- patterns -----------------------------------------------------

	fn format_pattern(&self, p: &PatternNode) -> Doc {
		match &p.kind {
			PatternKind::Identifier(ident) => text(ident.name.clone()),
			PatternKind::Underscore => text("_"),
			PatternKind::Literal(lit) => self.format_literal(lit),
			PatternKind::Constructor(head, args) => {
				let mut parts: Vec<Doc> = vec![text(head.dotted())];
				for a in args {
					parts.push(text(" "));
					parts.push(self.format_pattern_arg(a));
				}
				concat(parts)
			}
			PatternKind::Tuple(items) => {
				let docs: Vec<Doc> = items.iter().map(|p| self.format_pattern(p)).collect();
				bracketed("(", ")", docs)
			}
			PatternKind::Record { fields, rest } => {
				let mut docs: Vec<Doc> = fields
					.iter()
					.map(|(name, pat)| {
						// Field shorthand: render `{a: a}` as `{a}` when
						// the sub-pattern is an identifier-bind of the
						// same name.
						if let PatternKind::Identifier(ident) = &pat.kind {
							if ident.name == name.name {
								return text(name.name.clone());
							}
						}
						concat(vec![
							text(name.name.clone()),
							text(": "),
							self.format_pattern(pat),
						])
					})
					.collect();
				if let Some(rp) = rest {
					let rest_text = match &rp.binding {
						Some(ident) => format!("...{}", ident.name),
						None => "...".into(),
					};
					docs.push(text(rest_text));
				}
				if docs.is_empty() {
					return text("{}");
				}
				bracketed_collection("{", "}", docs)
			}
			PatternKind::List { items, rest } => {
				let mut docs: Vec<Doc> = items.iter().map(|p| self.format_pattern(p)).collect();
				if let Some(rp) = rest {
					let rest_text = match &rp.binding {
						Some(ident) => format!("...{}", ident.name),
						None => "...".into(),
					};
					docs.push(text(rest_text));
				}
				if docs.is_empty() {
					return text("[]");
				}
				bracketed("[", "]", docs)
			}
			PatternKind::Interpolation(parts) => self.format_interpolation(parts),
		}
	}

	// Format a pattern in constructor-argument position. The parser only
	// accepts "atoms" there (`Parser::parse_pattern_atom`): a record pattern
	// or a nested constructor-with-args isn't one, so it must be wrapped in
	// parens — `found ({name: n})`, `some (node l r)` — or the formatted
	// output would silently change meaning or fail to reparse (a bare `{`
	// reads as the match body; a bare nested constructor's args flatten into
	// the outer constructor's arg list). Lists, tuples (their own parens),
	// literals, identifiers and `_` are all atoms and need no wrapping.
	fn format_pattern_arg(&self, p: &PatternNode) -> Doc {
		if pattern_needs_parens_as_arg(p) {
			concat(vec![text("("), self.format_pattern(p), text(")")])
		} else {
			self.format_pattern(p)
		}
	}

	// --- type expressions ---------------------------------------------

	fn format_type_expr(&self, t: &TypeExprNode) -> Doc {
		match &t.kind {
			TypeExprKind::Single(ident) => self.format_type_identifier(ident),
			TypeExprKind::Func(params, ret) => {
				let mut parts: Vec<Doc> = vec![text("fun")];
				for p in params {
					parts.push(text(" "));
					parts.push(self.format_type_expr(p));
				}
				parts.push(text(" -> "));
				parts.push(self.format_type_expr(ret));
				concat(parts)
			}
			TypeExprKind::Tuple(items) => {
				let docs: Vec<Doc> = items.iter().map(|t| self.format_type_expr(t)).collect();
				bracketed("(", ")", docs)
			}
			TypeExprKind::Record(fields) => {
				if fields.is_empty() {
					return text("{}");
				}
				let docs: Vec<Doc> = fields
					.iter()
					.map(|(name, ty)| {
						concat(vec![
							text(name.name.clone()),
							text(" :: "),
							self.format_type_expr(ty),
						])
					})
					.collect();
				bracketed_collection("{", "}", docs)
			}
			TypeExprKind::EmptyTuple => text("()"),
			TypeExprKind::Grouping(inner) => {
				concat(vec![text("("), self.format_type_expr(inner), text(")")])
			}
		}
	}

	fn format_type_identifier(&self, t: &TypeIdentifierNode) -> Doc {
		let mut parts: Vec<Doc> = Vec::new();
		if let Some(m) = &t.module {
			parts.push(text(m.name.clone()));
			parts.push(text("."));
		}
		parts.push(text(t.name.clone()));
		for g in &t.generics {
			parts.push(text(" "));
			parts.push(self.format_type_expr(g));
		}
		concat(parts)
	}
}

// Canonical import-group rank: stdlib (`std.`) sorts first, then package
// (`pkg.`) imports, then user imports (any other leading segment). Within a
// group, imports are ordered by segment count then alphabetically.
fn use_group_rank(u: &UseNode) -> u8 {
	match u.path.first().map(|s| s.name.as_str()) {
		Some("std") => 0,
		Some("pkg") => 1,
		_ => 2,
	}
}

// The precedence (binding power) of an expression's top-level operator,
// mirroring `Operator::infix_binding_power` / `prefix_binding_power`. An
// expression can appear unparenthesized in a context requiring `min_prec`
// iff `expr_prec(e) >= min_prec`. Self-delimiting primaries (literals,
// identifiers, `fun`/`if`/`when`/`while`, lists/records/tuples, interpolations,
// regexes, ...) bind maximally tight — they never need parens. `let`/`try`
// are the exception: they bind their RHS greedily and aren't brace-delimited,
// so they're only safe bare at statement position (precedence 0).
fn expr_prec(e: &ExprNode) -> u8 {
	use ExprKind::*;
	match &e.kind {
		Grouping(inner) => expr_prec(inner),
		BinaryOperation { op, .. } => op_prec(&op.kind),
		UnaryOperation { op, .. } => prefix_prec(op),
		Call(_) => 90,
		FieldAccess { .. } | ElementAccess { .. } => 100,
		Let(_) | Try(_) | Defer(_) => 0,
		_ => u8::MAX,
	}
}

// Whether a pattern needs wrapping parens when it appears as a constructor
// argument (mirrors `Parser::parse_pattern_atom`). Records aren't accepted
// bare in that position, and a nested constructor (which always carries
// args — a zero-arg variant parses as an `Identifier`) would otherwise
// flatten into the outer constructor's arg list. Everything else — lists,
// tuples, literals, identifiers, `_`, interpolations — is already an atom.
fn pattern_needs_parens_as_arg(p: &PatternNode) -> bool {
	matches!(
		&p.kind,
		PatternKind::Record { .. } | PatternKind::Constructor(..)
	)
}

// `if cond { }`/`while cond { }` is the canonical surface form of an `is true`
// match, so the formatter drops a literal `true` pattern. This both renders the
// shorter form and keeps formatting idempotent with the parser, which
// synthesizes exactly this pattern when `is` is omitted.
fn pattern_is_true(p: &PatternNode) -> bool {
	matches!(
		&p.kind,
		PatternKind::Literal(LiteralNode {
			kind: LiteralKind::Bool(true),
			..
		})
	)
}

// An `if`/`while` subject is parsed under brace restriction: the first
// top-level `{` opens the body, never a record literal. So a subject whose
// printed form would expose a top-level `{` must be parenthesized to
// round-trip. Groupings are transparent here because `fmt` strips redundant
// parens, so the guard is re-derived from structure rather than trusting source
// parens. Record/record-update/`fun`/block-headed expressions print a `{`
// directly; `(...)`/`[...]` protect any braces nested inside; compositional
// forms expose a brace iff a bare child does.
// Whether `e` continues the brace-sensitive spine of an `if`/`while` subject:
// its syntax prints at least one child bare (no enclosing delimiter), so a
// record reached through that child is still exposed to the body-opening `{`.
// Records/record-updates are handled separately (they're the thing wrapped);
// every other kind is either an atom or wraps its children in a delimiter or
// keyword, ending the spine.
fn is_brace_spine(e: &ExprNode) -> bool {
	use ExprKind::*;
	matches!(
		&e.kind,
		Call(..)
			| BinaryOperation { .. }
			| UnaryOperation { .. }
			| FieldAccess { .. }
			| ElementAccess { .. }
			| Defer(..)
			| Try(..)
			| Let(..)
	)
}

// Binding-power level of an infix operator (see `operator.rs`). Same-level
// operators share a number; associativity is applied by the caller.
fn op_prec(op: &Operator) -> u8 {
	use Operator::*;
	match op {
		Chain => 0,
		Range => 10,
		LogicalOr => 20,
		NullCoalescing => 21,
		LogicalAnd => 30,
		Equality | Inequality => 40,
		LessThan | LessThanEquals | GreaterThan | GreaterThanEquals => 50,
		Addition | SubtractionOrNegation | Concat => 60,
		Multiplication | Division | Remainder => 70,
		Exponentiation => 80,
		LogicalNot => 35,
		FunctionCall => 90,
		FieldAccess | IndexAccess => 100,
	}
}

// `**` and `??` are the only right-associative operators.
fn is_right_assoc(op: &Operator) -> bool {
	matches!(op, Operator::Exponentiation | Operator::NullCoalescing)
}

// Binding power of a prefix operator's operand. Only `-` (negation) is
// reachable in practice; `!` never parses as a prefix today but is mapped
// for completeness.
fn prefix_prec(op: &Operator) -> u8 {
	match op {
		Operator::LogicalNot => 35,
		_ => 75,
	}
}

// `(a, b, c)` / `[a, b, c]` — always inline. Used for tuple and list *patterns*,
// where line breaks aren't idiomatic. (Value literals go through
// `bracketed_collection`, which wraps when wide.)
fn bracketed(open: &str, close: &str, items: Vec<Doc>) -> Doc {
	concat(vec![
		text(open.to_string()),
		join(text(", "), items),
		text(close.to_string()),
	])
}

// `[a, b, c]` / `{a: 1, b: 2}` / `(a, b, c)` — flat with comma+space, or one
// item per line. The choice is made by the surrounding Group based on width.
// Pluma list/record/tuple literals are comma-separated in either layout (unlike
// newline-separated enum variants), so the wrapped form keeps a comma between
// items — a bare trailing newline is what the parser would reject. When it
// breaks across lines it also adds a trailing comma on the last item; the flat
// form (`[a, b]`) has none.
fn bracketed_collection(open: &str, close: &str, items: Vec<Doc>) -> Doc {
	let sep = if_flat(text(", "), concat(vec![text(","), line()]));
	let n = items.len();
	let mut inner: Vec<Doc> = Vec::with_capacity(n * 2 + 1);
	for (i, item) in items.into_iter().enumerate() {
		if i > 0 {
			inner.push(sep.clone());
		}
		inner.push(item);
	}
	// Trailing comma, but only once the group has broken across lines.
	if n > 0 {
		inner.push(if_flat(text(""), text(",")));
	}
	group(concat(vec![
		text(open.to_string()),
		nest(concat(vec![softbreak(), concat(inner)])),
		softbreak(),
		text(close.to_string()),
	]))
}

fn escape_string(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for c in s.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\t' => out.push_str("\\t"),
			'\r' => out.push_str("\\r"),
			'\n' => out.push_str("\\n"),
			_ => out.push(c),
		}
	}
	out
}

fn escape_bytes(b: &[u8]) -> String {
	let mut out = String::with_capacity(b.len());
	for &byte in b {
		match byte {
			b'\\' => out.push_str("\\\\"),
			b'\'' => out.push_str("\\'"),
			b'\t' => out.push_str("\\t"),
			b'\r' => out.push_str("\\r"),
			b'\n' => out.push_str("\\n"),
			0x20..=0x7e => out.push(byte as char),
			_ => out.push_str(&format!("\\x{:02x}", byte)),
		}
	}
	out
}

fn format_float(f: f64) -> String {
	if f.is_nan() {
		return "nan".to_string();
	}
	if f.is_infinite() {
		return if f.is_sign_negative() { "-inf" } else { "inf" }.to_string();
	}
	let s = format!("{}", f);
	if s.contains('.') || s.contains('e') || s.contains('E') {
		s
	} else {
		format!("{}.0", s)
	}
}

// Render a duration (in nanoseconds) as canonical unit segments — largest unit
// first, each unit at most once, zero components dropped (e.g. 90_000_000_000 ->
// "1m30s"). This is exactly the form the parser accepts, so formatting is
// idempotent even when the source used a non-canonical spelling like `90s`.
fn format_duration(nanos: i64) -> String {
	if nanos == 0 {
		return "0s".to_string();
	}
	let (sign, mut rem): (&str, u128) = if nanos < 0 {
		("-", (nanos as i128).unsigned_abs())
	} else {
		("", nanos as u128)
	};
	const UNITS: [(u128, &str); 7] = [
		(86_400_000_000_000, "d"),
		(3_600_000_000_000, "h"),
		(60_000_000_000, "m"),
		(1_000_000_000, "s"),
		(1_000_000, "ms"),
		(1_000, "us"),
		(1, "ns"),
	];
	let mut out = String::from(sign);
	for (per, name) in UNITS {
		if rem >= per {
			out.push_str(&(rem / per).to_string());
			out.push_str(name);
			rem %= per;
		}
	}
	out
}
