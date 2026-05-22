use compiler::ast::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::doc::*;

pub(crate) struct Formatter<'a> {
	comments: &'a HashMap<usize, String>,
	consumed: RefCell<HashSet<usize>>,
}

impl<'a> Formatter<'a> {
	pub fn new(comments: &'a HashMap<usize, String>) -> Self {
		Self {
			comments,
			consumed: RefCell::new(HashSet::new()),
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

		// Imports: one per line, no blank between consecutive imports.
		for (i, u) in m.uses.iter().enumerate() {
			if i > 0 {
				parts.push(hardline());
			}
			parts.push(self.format_use(u));
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
		match &def.kind {
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
				concat(vec![
					text("def "),
					text(def.name.name.clone()),
					text(" = "),
					value_doc,
				])
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
			DefinitionKind::Test { description, body } => {
				let header = concat(vec![
					text("test "),
					text(format!("{:?}", description)),
					text(" {"),
				]);
				if body.is_empty() {
					return concat(vec![header, text("}")]);
				}
				concat(vec![
					header,
					nest(self.format_statements(body)),
					hardline(),
					text("}"),
				])
			}
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
			header.push(text(" where ("));
			let constraints: Vec<Doc> = inst
				.where_clause
				.iter()
				.map(|c| {
					concat(vec![
						text(c.trait_name.name.clone()),
						text(" "),
						text(c.param.name.clone()),
					])
				})
				.collect();
			header.push(join(text(", "), constraints));
			header.push(text(")"));
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
			prev_line = Some(m.range.end.line);
		}

		concat(vec![
			concat(header),
			nest(concat(body)),
			hardline(),
			text("}"),
		])
	}

	// --- expressions --------------------------------------------------

	fn format_expr(&self, e: &ExprNode) -> Doc {
		use ExprKind::*;
		match &e.kind {
			Literal(lit) => self.format_literal(lit),
			Identifier(ident) => text(ident.name.clone()),
			EmptyTuple => text("()"),
			Grouping(inner) => concat(vec![text("("), self.format_expr(inner), text(")")]),
			BinaryOperation { op, left, right } => concat(vec![
				self.format_expr(left),
				text(" "),
				text(format!("{}", op.kind)),
				text(" "),
				self.format_expr(right),
			]),
			UnaryOperation { op, right } => {
				concat(vec![text(format!("{}", op)), self.format_expr(right)])
			}
			ElementAccess { receiver, index } => concat(vec![
				self.format_expr(receiver),
				text("."),
				text(index.to_string()),
			]),
			FieldAccess { receiver, field } => concat(vec![
				self.format_expr(receiver),
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
			Fun(fun) => self.format_fun(fun),
			Call(call) => self.format_call(call),
			Let(l) => self.format_let(l),
			Try(t) => self.format_try(t),
			Tuple(items) => self.format_tuple(items),
			List(items) => self.format_list(items),
			Record(fields) => self.format_record(fields),
			Interpolation(parts) => self.format_interpolation(parts),
			Regex(r) => self.format_regex_literal(r),
			If(i) => self.format_if(i),
			When(w) => self.format_when(w),
			While(w) => self.format_while(w),
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
				nest(concat(vec![line(), self.format_expr(only)])),
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
			parts.push(self.format_expr(expr));
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

	fn format_call(&self, call: &CallNode) -> Doc {
		// Calls are whitespace-separated callee + args. We keep them flat
		// when they fit, otherwise wrap each argument onto its own line at
		// the same indent + 1 level.
		let mut parts: Vec<Doc> = vec![self.format_expr(&call.callee)];
		for arg in &call.args {
			parts.push(group(nest(concat(vec![line(), self.format_expr(arg)]))));
		}
		group(concat(parts))
	}

	fn format_let(&self, l: &LetNode) -> Doc {
		concat(vec![
			text("let "),
			self.format_pattern(&l.pattern),
			text(" = "),
			self.format_expr(&l.value),
		])
	}

	// `try` mirrors `let`'s shape: `try Pattern = Expr`. The continuation
	// (`rest`) is rendered as inline siblings — at the source level a try
	// has no braces around what follows, just subsequent expressions in
	// the enclosing block.
	fn format_try(&self, t: &TryNode) -> Doc {
		let mut parts: Vec<Doc> = vec![
			text("try "),
			self.format_pattern(&t.pattern),
			text(" = "),
			self.format_expr(&t.value),
		];
		for e in &t.rest {
			parts.push(hardline());
			parts.push(self.format_expr(e));
		}
		concat(parts)
	}

	fn format_tuple(&self, items: &[ExprNode]) -> Doc {
		if items.is_empty() {
			return text("()");
		}
		let docs: Vec<Doc> = items.iter().map(|e| self.format_expr(e)).collect();
		bracketed("(", ")", docs)
	}

	fn format_list(&self, items: &[ExprNode]) -> Doc {
		if items.is_empty() {
			return text("[]");
		}
		let docs: Vec<Doc> = items.iter().map(|e| self.format_expr(e)).collect();
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

	fn format_if(&self, i: &IfNode) -> Doc {
		// `if` always lays out multi-line, mirroring `when`:
		//
		//     if SUBJECT is PATTERN {
		//         body
		//     } else {
		//         else_body
		//     }
		//
		// `} else {` sits on the same line as the body's closing brace.
		let mut parts: Vec<Doc> = vec![
			text("if "),
			self.format_expr(&i.subject),
			text(" is "),
			self.format_pattern(&i.pattern),
			text(" {"),
			nest(self.format_statements(&i.body)),
			hardline(),
		];
		if let Some(else_body) = &i.else_body {
			parts.push(text("} else {"));
			parts.push(nest(self.format_statements(else_body)));
			parts.push(hardline());
		}
		parts.push(text("}"));
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
		concat(vec![
			text("while "),
			self.format_expr(&w.subject),
			text(" is "),
			self.format_pattern(&w.pattern),
			text(" "),
			self.format_block(&w.body),
		])
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
				nest(concat(vec![line(), self.format_expr(only)])),
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
			PatternKind::Constructor(name, args) => {
				let mut parts: Vec<Doc> = vec![text(name.name.clone())];
				for a in args {
					parts.push(text(" "));
					parts.push(self.format_pattern(a));
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

// `(a, b, c)` — always inline (used for tuples, where line breaks aren't
// idiomatic in Pluma).
fn bracketed(open: &str, close: &str, items: Vec<Doc>) -> Doc {
	concat(vec![
		text(open.to_string()),
		join(text(", "), items),
		text(close.to_string()),
	])
}

// `[a, b, c]` / `{a: 1, b: 2}` — flat with comma+space, or one item per line
// with no commas. The choice is made by the surrounding Group based on width.
fn bracketed_collection(open: &str, close: &str, items: Vec<Doc>) -> Doc {
	let sep = if_flat(text(", "), line());
	let n = items.len();
	let mut inner: Vec<Doc> = Vec::with_capacity(n * 2);
	for (i, item) in items.into_iter().enumerate() {
		if i > 0 {
			inner.push(sep.clone());
		}
		inner.push(item);
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
