//! Syntax classification: assign each piece of source a highlight class. One
//! shared classifier drives both the LSP's semantic tokens and the docs site's
//! code blocks, so editor and website highlight identically.
//!
//! Two passes. A lexer pass classifies what the tokenizer already knows —
//! keywords, strings, numbers, comments, operators. A parse-only AST pass then
//! adds the richer identifier roles the lexer can't see — which names are
//! functions vs variables vs types vs enum variants vs namespaces. The AST pass
//! overrides the lexer where they overlap. We deliberately skip the full
//! analyzer: the parser alone gives enough syntactic context, and it needs no
//! resolved imports from disk (so it works on a bare snippet).

use crate::ast::*;
use crate::{Diagnostic, Module, Range, Token, Tokenizer};
use std::collections::HashSet;
use std::path::PathBuf;

/// A highlight class. Ordering matches the LSP semantic-token legend, so the LSP
/// adapter can use `class as u32` directly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
	Comment,
	Function,
	Variable,
	String,
	Number,
	Regexp,
	Keyword,
	Operator,
	Type,
	Parameter,
	Property,
	Enum,
	EnumMember,
	Namespace,
	Interface,
	TypeParameter,
}

impl Class {
	/// A stable kebab-case name, used as a CSS class on the docs site.
	pub fn name(self) -> &'static str {
		match self {
			Class::Comment => "comment",
			Class::Function => "function",
			Class::Variable => "variable",
			Class::String => "string",
			Class::Number => "number",
			Class::Regexp => "regexp",
			Class::Keyword => "keyword",
			Class::Operator => "operator",
			Class::Type => "type",
			Class::Parameter => "parameter",
			Class::Property => "property",
			Class::Enum => "enum",
			Class::EnumMember => "enum-member",
			Class::Namespace => "namespace",
			Class::Interface => "interface",
			Class::TypeParameter => "type-parameter",
		}
	}
}

/// A classified slice of one source line.
#[derive(Clone, Copy)]
pub struct HlToken {
	pub line: usize,
	pub col: usize,
	pub len: usize,
	pub class: Class,
}

/// Classify `source` into highlight tokens: sorted by position, non-overlapping,
/// each within a single line. The shared entry point for the LSP and the docs.
pub fn classify(source: &[u8]) -> Vec<HlToken> {
	let mut tokens: Vec<HlToken> = Vec::new();

	collect_from_lexer(source, &mut tokens);

	let mut module = Module::new("<highlight>".to_string(), PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);
	if let Some(ast) = module.ast.as_ref() {
		let mut walker = AstWalker::new(ast);
		walker.walk_module(ast, &mut tokens);
	}

	dedup(tokens)
}

/// A run of text with an optional class — `None` for unclassified spans
/// (whitespace, punctuation, anything the classifier skipped). Concatenating the
/// `text`s reproduces `source` exactly. For rendering code blocks as HTML.
pub struct Span {
	pub class: Option<Class>,
	pub text: String,
}

/// Partition `source` into consecutive classified/plain spans covering every
/// byte, so a renderer can wrap each classified run in its class and leave the
/// rest as plain text.
pub fn spans(source: &str) -> Vec<Span> {
	let starts = line_starts(source);
	let mut ranges: Vec<(usize, usize, Class)> = classify(source.as_bytes())
		.iter()
		.filter_map(|t| {
			let start = starts.get(t.line)? + t.col;
			Some((start, start + t.len, t.class))
		})
		.collect();
	ranges.sort_by_key(|r| r.0);

	let mut out: Vec<Span> = Vec::new();
	let mut cursor = 0usize;
	for (start, end, class) in ranges {
		// Skip anything out of bounds or overlapping what we've already emitted.
		if start < cursor || end > source.len() || start > end {
			continue;
		}
		if start > cursor {
			out.push(Span {
				class: None,
				text: source[cursor..start].to_string(),
			});
		}
		out.push(Span {
			class: Some(class),
			text: source[start..end].to_string(),
		});
		cursor = end;
	}
	if cursor < source.len() {
		out.push(Span {
			class: None,
			text: source[cursor..].to_string(),
		});
	}
	out
}

fn collect_from_lexer(source: &[u8], out: &mut Vec<HlToken>) {
	let owned = source.to_vec();
	let mut tokenizer = Tokenizer::from_source(&owned);
	let mut line = 0usize;
	let mut line_start = 0usize;

	loop {
		match tokenizer.next() {
			None => break,
			Some(tok) => {
				if tok.is_line_break() {
					line_start = tok.get_span().1;
					line += 1;
					continue;
				}

				let (mut start, mut end) = tok.get_span();
				// The tokenizer's StringLiteral span covers only the content
				// between the quotes. Absorb the delimiting `"` on each side so
				// the quotes highlight too. Interpolation splits a string into
				// pieces bounded by `$(`/`)`, so only take a neighbor that's
				// actually a quote — the opening quote belongs to the first
				// piece, the closing quote to the last.
				if matches!(tok, Token::StringLiteral(..)) {
					if start > line_start && source[start - 1] == b'"' {
						start -= 1;
					}
					if end < source.len() && source[end] == b'"' {
						end += 1;
					}
				}
				if start < line_start || end < start {
					continue;
				}
				if let Some(class) = lexer_class(&tok) {
					out.push(HlToken {
						line,
						col: start - line_start,
						len: end - start,
						class,
					});
				}
			}
		}
	}
}

fn lexer_class(t: &Token) -> Option<Class> {
	use Token::*;
	Some(match t {
		Comment(..) => Class::Comment,
		StringLiteral(..) => Class::String,
		// `$(` and `)` around interpolations: highlight as operator so they're
		// visibly distinct from the surrounding string.
		InterpolationStart(..) | InterpolationEnd(..) => Class::Operator,
		DecimalDigits(..) | HexDigits(..) | OctalDigits(..) | BinaryDigits(..) => Class::Number,
		DurationLiteral(..) => Class::Number,
		// `true`/`false` are bool-constructor tokens, but render best as keywords.
		BoolTrue(..) | BoolFalse(..) => Class::Keyword,
		KeywordAlias(..) | KeywordAnd(..) | KeywordAs(..) | KeywordBuiltin(..) | KeywordDef(..)
		| KeywordDefer(..) | KeywordElse(..) | KeywordEnum(..) | KeywordFun(..) | KeywordIf(..)
		| KeywordImplement(..) | KeywordIn(..) | KeywordIs(..) | KeywordLet(..) | KeywordManual(..)
		| KeywordOpaque(..) | KeywordOr(..) | KeywordPublic(..) | KeywordRemote(..)
		| KeywordScope(..) | KeywordTrait(..) | KeywordTry(..) | KeywordUse(..) | KeywordUsing(..)
		| KeywordWhen(..) | KeywordWhere(..) | KeywordWhile(..) => Class::Keyword,
		Arrow(..)
		| Bang(..)
		| BangEqual(..)
		| DoubleAnd(..)
		| DoubleColon(..)
		| DoubleDot(..)
		| DoubleEqual(..)
		| DoubleForwardSlash(..)
		| DoubleLeftAngle(..)
		| DoublePipe(..)
		| DoublePlus(..)
		| DoubleQuestion(..)
		| DoubleRightAngle(..)
		| TripleRightAngle(..)
		| DoubleStar(..)
		| Equal(..)
		| ForwardSlash(..)
		| LeftAngle(..)
		| LeftAngleEqual(..)
		| Minus(..)
		| UnaryMinus(..)
		| Percent(..)
		| Pipe(..)
		| PipeArrow(..)
		| Plus(..)
		| Question(..)
		| RightAngle(..)
		| RightAngleEqual(..)
		| Star(..)
		| Tilde(..)
		| And(..)
		| Backtick(..) => Class::Operator,
		// Identifiers and paths are left to the AST pass. Pure punctuation
		// (parens, braces, commas, dots, underscore, indent/outdent) is
		// unhighlighted on purpose.
		_ => return None,
	})
}

// Sort by position; collapse exact-position duplicates (the AST pass, appended
// after the lexer, wins) and drop entries that start inside the previous one
// (callers — and the LSP — need non-overlapping tokens).
fn dedup(mut tokens: Vec<HlToken>) -> Vec<HlToken> {
	tokens.sort_by_key(|t| (t.line, t.col));

	let mut out: Vec<HlToken> = Vec::with_capacity(tokens.len());
	for tok in tokens {
		while let Some(last) = out.last().copied() {
			if last.line == tok.line && last.col == tok.col {
				out.pop();
				continue;
			}
			if last.line == tok.line && last.col + last.len > tok.col {
				// Token starts inside the previous one — skip it.
				break;
			}
			break;
		}
		if let Some(last) = out.last() {
			if last.line == tok.line && last.col == tok.col {
				continue;
			}
			if last.line == tok.line && last.col + last.len > tok.col {
				continue;
			}
		}
		out.push(tok);
	}
	out
}

fn line_starts(source: &str) -> Vec<usize> {
	let mut starts = vec![0];
	for (i, b) in source.bytes().enumerate() {
		if b == b'\n' {
			starts.push(i + 1);
		}
	}
	starts
}

// -- AST walker -----------------------------------------------------------

struct AstWalker {
	// Names bound by `use` at this module's top level — when seen as a
	// FieldAccess receiver they classify as Namespace.
	module_names: HashSet<String>,
	// Names bound by top-level `enum` defs — when seen as a FieldAccess
	// receiver they classify as Enum (e.g. `color.red`).
	enum_names: HashSet<String>,
}

impl AstWalker {
	fn new(ast: &ModuleNode) -> Self {
		let mut module_names = HashSet::new();
		for u in &ast.uses {
			module_names.insert(u.local_name().name.clone());
		}
		let mut enum_names = HashSet::new();
		for def in &ast.body {
			if matches!(def.kind, DefinitionKind::Enum(_)) {
				enum_names.insert(def.name.name.clone());
			}
		}
		Self {
			module_names,
			enum_names,
		}
	}

	fn walk_module(&mut self, m: &ModuleNode, out: &mut Vec<HlToken>) {
		for u in &m.uses {
			self.walk_use(u, out);
		}
		for def in &m.body {
			self.walk_def(def, out);
		}
	}

	fn walk_use(&mut self, u: &UseNode, out: &mut Vec<HlToken>) {
		for seg in &u.path {
			emit(out, &seg.range, Class::Namespace, seg.name.len());
		}
		if let Some(alias) = &u.alias {
			emit(out, &alias.range, Class::Namespace, alias.name.len());
		}
	}

	fn walk_def(&mut self, d: &DefinitionNode, out: &mut Vec<HlToken>) {
		match &d.kind {
			DefinitionKind::Expr(expr) => {
				let kind = if let ExprKind::Fun(_) = &expr.kind {
					Class::Function
				} else {
					Class::Variable
				};
				emit(out, &d.name.range, kind, d.name.name.len());
				self.walk_expr(expr, out);
			}
			DefinitionKind::Alias(ty_expr) => {
				emit(out, &d.name.range, Class::Type, d.name.name.len());
				self.walk_type_expr(ty_expr, out);
			}
			DefinitionKind::Enum(en) => {
				emit(out, &d.name.range, Class::Enum, d.name.name.len());
				for p in &en.params {
					emit(out, &p.range, Class::TypeParameter, p.name.len());
				}
				for v in &en.variants {
					emit(out, &v.name.range, Class::EnumMember, v.name.name.len());
					if let Some(params) = &v.params {
						for p in params {
							self.walk_type_expr(p, out);
						}
					}
				}
			}
			DefinitionKind::Trait(t) => {
				emit(out, &d.name.range, Class::Interface, d.name.name.len());
				emit(
					out,
					&t.param.range,
					Class::TypeParameter,
					t.param.name.len(),
				);
				for m in &t.methods {
					emit(out, &m.name.range, Class::Function, m.name.name.len());
					self.walk_type_expr(&m.signature, out);
					if let Some(default) = &m.default {
						self.walk_expr(default, out);
					}
				}
			}
			DefinitionKind::Instance(inst) => {
				emit(
					out,
					&inst.trait_name.range,
					Class::Interface,
					inst.trait_name.name.len(),
				);
				self.walk_type_expr(&inst.head, out);
				for c in &inst.where_clause {
					emit(
						out,
						&c.trait_name.range,
						Class::Interface,
						c.trait_name.name.len(),
					);
					emit(
						out,
						&c.param.range,
						Class::TypeParameter,
						c.param.name.len(),
					);
				}
				for method in &inst.methods {
					self.walk_def(method, out);
				}
			}
		}
	}

	fn walk_expr(&mut self, e: &ExprNode, out: &mut Vec<HlToken>) {
		match &e.kind {
			ExprKind::Identifier(ident) => {
				let kind = if self.module_names.contains(&ident.name) {
					Class::Namespace
				} else if self.enum_names.contains(&ident.name) {
					Class::Enum
				} else {
					Class::Variable
				};
				emit(out, &ident.range, kind, ident.name.len());
			}
			ExprKind::Literal(_) => {}
			ExprKind::Regex(r) => {
				emit(out, &r.range, Class::Regexp, range_len_hint(&r.range));
			}
			ExprKind::BinaryOperation { left, right, .. } => {
				self.walk_expr(left, out);
				self.walk_expr(right, out);
			}
			ExprKind::UnaryOperation { right, .. } => {
				self.walk_expr(right, out);
			}
			ExprKind::ElementAccess { receiver, .. } => {
				self.walk_expr(receiver, out);
			}
			ExprKind::FieldAccess { receiver, field } => {
				let (receiver_kind, field_kind) = match &receiver.kind {
					ExprKind::Identifier(id) if self.module_names.contains(&id.name) => {
						(Some(Class::Namespace), Class::Property)
					}
					ExprKind::Identifier(id) if self.enum_names.contains(&id.name) => {
						(Some(Class::Enum), Class::EnumMember)
					}
					_ => (None, Class::Property),
				};
				if let Some(k) = receiver_kind {
					if let ExprKind::Identifier(id) = &receiver.kind {
						emit(out, &id.range, k, id.name.len());
					}
				} else {
					self.walk_expr(receiver, out);
				}
				emit(out, &field.range, field_kind, field.name.len());
			}
			ExprKind::Fun(f) => self.walk_fun(f, out),
			ExprKind::Call(c) => {
				match &c.callee.kind {
					ExprKind::Identifier(id) => {
						emit(out, &id.range, Class::Function, id.name.len());
					}
					_ => self.walk_expr(&c.callee, out),
				}
				for arg in &c.args {
					self.walk_expr(arg, out);
				}
			}
			ExprKind::EmptyTuple => {}
			ExprKind::Grouping(inner) | ExprKind::Defer(inner) => self.walk_expr(inner, out),
			ExprKind::Interpolation(parts) => {
				for p in parts {
					self.walk_expr(p, out);
				}
			}
			ExprKind::Let(l) => {
				match &l.pattern.kind {
					PatternKind::Identifier(id) => {
						let kind = if let ExprKind::Fun(_) = &l.value.kind {
							Class::Function
						} else {
							Class::Variable
						};
						emit(out, &id.range, kind, id.name.len());
					}
					_ => self.walk_pattern(&l.pattern, out),
				}
				self.walk_expr(&l.value, out);
			}
			ExprKind::Record(fields) => {
				for (name, value) in fields {
					emit(out, &name.range, Class::Property, name.name.len());
					self.walk_expr(value, out);
				}
			}
			ExprKind::RecordUpdate { base, fields } => {
				self.walk_expr(base, out);
				for (name, value) in fields {
					emit(out, &name.range, Class::Property, name.name.len());
					self.walk_expr(value, out);
				}
			}
			ExprKind::Tuple(elements) => {
				for el in elements {
					self.walk_expr(el, out);
				}
			}
			ExprKind::List(items) => {
				for item in items {
					self.walk_expr(item.expr(), out);
				}
			}
			ExprKind::If(i) => {
				self.walk_expr(&i.subject, out);
				self.walk_pattern(&i.pattern, out);
				for e in &i.body {
					self.walk_expr(e, out);
				}
				if let Some(else_body) = &i.else_body {
					for e in else_body {
						self.walk_expr(e, out);
					}
				}
			}
			ExprKind::When(w) => {
				self.walk_expr(&w.subject, out);
				for case in &w.cases {
					self.walk_pattern(&case.pattern, out);
					for e in &case.body {
						self.walk_expr(e, out);
					}
				}
			}
			ExprKind::While(w) => {
				self.walk_expr(&w.subject, out);
				self.walk_pattern(&w.pattern, out);
				for e in &w.body {
					self.walk_expr(e, out);
				}
			}
			ExprKind::Using { body, .. } => {
				for e in body {
					self.walk_expr(e, out);
				}
			}
			ExprKind::ImplicitMember { .. } => {}
			ExprKind::Scope(s) => {
				for e in &s.body {
					self.walk_expr(e, out);
				}
			}
			ExprKind::NamespaceAccess(_) => {}
			ExprKind::Try(t) => {
				match &t.pattern.kind {
					PatternKind::Identifier(id) => {
						emit(out, &id.range, Class::Variable, id.name.len());
					}
					_ => self.walk_pattern(&t.pattern, out),
				}
				self.walk_expr(&t.value, out);
				for e in &t.rest {
					self.walk_expr(e, out);
				}
			}
			ExprKind::Builtin(_) => {}
		}
	}

	fn walk_fun(&mut self, f: &FunNode, out: &mut Vec<HlToken>) {
		for p in &f.params {
			emit(out, &p.ident.range, Class::Parameter, p.ident.name.len());
		}
		for e in &f.body {
			self.walk_expr(e, out);
		}
	}

	fn walk_pattern(&mut self, p: &PatternNode, out: &mut Vec<HlToken>) {
		match &p.kind {
			PatternKind::Identifier(id) => {
				emit(out, &id.range, Class::Variable, id.name.len());
			}
			PatternKind::Constructor(head, inner) => {
				if let Some(module) = &head.module {
					emit(out, &module.range, Class::Namespace, module.name.len());
				}
				if let Some(enum_name) = &head.enum_name {
					emit(out, &enum_name.range, Class::Enum, enum_name.name.len());
				}
				emit(
					out,
					&head.variant.range,
					Class::EnumMember,
					head.variant.name.len(),
				);
				for ip in inner {
					self.walk_pattern(ip, out);
				}
			}
			PatternKind::Tuple(items) => {
				for ip in items {
					self.walk_pattern(ip, out);
				}
			}
			PatternKind::Record { fields, rest } => {
				for (name, sub) in fields {
					emit(out, &name.range, Class::Property, name.name.len());
					self.walk_pattern(sub, out);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						emit(out, &name.range, Class::Variable, name.name.len());
					}
				}
			}
			PatternKind::List { items, rest } => {
				for ip in items {
					self.walk_pattern(ip, out);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						emit(out, &name.range, Class::Variable, name.name.len());
					}
				}
			}
			PatternKind::Underscore | PatternKind::Literal(_) => {}
			PatternKind::Interpolation(parts) => {
				for e in parts {
					self.walk_expr(e, out);
				}
			}
		}
	}

	fn walk_type_expr(&mut self, t: &TypeExprNode, out: &mut Vec<HlToken>) {
		match &t.kind {
			TypeExprKind::Single(id) => {
				let name_line = id.range.start.line;
				let name_col = match &id.module {
					Some(module) => {
						emit(out, &module.range, Class::Namespace, module.name.len());
						module.range.end.col + 1
					}
					None => id.range.start.col,
				};
				out.push(HlToken {
					line: name_line,
					col: name_col,
					len: id.name.len(),
					class: Class::Type,
				});
				for g in &id.generics {
					self.walk_type_expr(g, out);
				}
			}
			TypeExprKind::Func(params, ret) => {
				for p in params {
					self.walk_type_expr(p, out);
				}
				self.walk_type_expr(ret, out);
			}
			TypeExprKind::Tuple(items) => {
				for it in items {
					self.walk_type_expr(it, out);
				}
			}
			TypeExprKind::Record(fields) => {
				for (name, ty) in fields {
					emit(out, &name.range, Class::Property, name.name.len());
					self.walk_type_expr(ty, out);
				}
			}
			TypeExprKind::EmptyTuple => {}
			TypeExprKind::Grouping(inner) => self.walk_type_expr(inner, out),
		}
	}
}

// Emit a single-line token. Multi-line ranges are skipped — both the LSP (which
// requires single-line tokens) and the docs partition (which then leaves the
// region plain) tolerate that.
fn emit(out: &mut Vec<HlToken>, range: &Range, class: Class, len: usize) {
	if range.start.line != range.end.line {
		return;
	}
	out.push(HlToken {
		line: range.start.line,
		col: range.start.col,
		len,
		class,
	});
}

fn range_len_hint(range: &Range) -> usize {
	if range.start.line == range.end.line {
		range.end.col.saturating_sub(range.start.col)
	} else {
		0
	}
}
