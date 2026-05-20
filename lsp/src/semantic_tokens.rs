use compiler::ast::*;
use compiler::{Diagnostic, Module, Token, Tokenizer};
use std::collections::HashSet;
use std::path::PathBuf;
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType};

pub const TOKEN_TYPES: &[SemanticTokenType] = &[
	SemanticTokenType::COMMENT,        // 0
	SemanticTokenType::FUNCTION,       // 1
	SemanticTokenType::VARIABLE,       // 2
	SemanticTokenType::STRING,         // 3
	SemanticTokenType::NUMBER,         // 4
	SemanticTokenType::REGEXP,         // 5
	SemanticTokenType::KEYWORD,        // 6
	SemanticTokenType::OPERATOR,       // 7
	SemanticTokenType::TYPE,           // 8
	SemanticTokenType::PARAMETER,      // 9
	SemanticTokenType::PROPERTY,       // 10
	SemanticTokenType::ENUM,           // 11
	SemanticTokenType::ENUM_MEMBER,    // 12
	SemanticTokenType::NAMESPACE,      // 13
	SemanticTokenType::INTERFACE,      // 14
	SemanticTokenType::TYPE_PARAMETER, // 15
];

const COMMENT: u32 = 0;
const FUNCTION: u32 = 1;
const VARIABLE: u32 = 2;
const STRING: u32 = 3;
const NUMBER: u32 = 4;
const REGEXP: u32 = 5;
const KEYWORD: u32 = 6;
const OPERATOR: u32 = 7;
const TYPE: u32 = 8;
const PARAMETER: u32 = 9;
const PROPERTY: u32 = 10;
const ENUM: u32 = 11;
const ENUM_MEMBER: u32 = 12;
const NAMESPACE: u32 = 13;
const INTERFACE: u32 = 14;
const TYPE_PARAMETER: u32 = 15;

#[derive(Clone, Copy)]
struct Abs {
	line: u32,
	col: u32,
	length: u32,
	kind: u32,
}

pub fn collect_semantic_tokens(source: &Vec<u8>) -> Vec<SemanticToken> {
	let mut tokens: Vec<Abs> = Vec::new();

	collect_from_lexer(source, &mut tokens);

	// Parse-only pass: rich identifier roles. We skip the full analyzer
	// because it needs the project's imports resolved from disk; the
	// parser alone already gives us the syntactic context we need for
	// most highlighting.
	let mut module = Module::new("<lsp>".to_string(), PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.clone(), &mut diagnostics);
	if let Some(ast) = module.ast.as_ref() {
		let mut walker = AstWalker::new(ast);
		walker.walk_module(ast, &mut tokens);
	}

	encode_deltas(tokens)
}

fn collect_from_lexer(source: &Vec<u8>, out: &mut Vec<Abs>) {
	let mut tokenizer = Tokenizer::from_source(source);
	let mut line = 0u32;
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

				let (start, end) = tok.get_span();
				// Defensive guard against malformed token spans — a crashing
				// LSP gives a much worse experience than a missed highlight.
				if start < line_start || end < start {
					continue;
				}
				if let Some(kind) = lexer_kind(&tok) {
					out.push(Abs {
						line,
						col: (start - line_start) as u32,
						length: (end - start) as u32,
						kind,
					});
				}
			}
		}
	}
}

fn lexer_kind(t: &Token) -> Option<u32> {
	use Token::*;
	Some(match t {
		Comment(..) => COMMENT,
		StringLiteral(..) => STRING,
		// `$(` and `)` around interpolations: highlight as operator so
		// they're visibly distinct from the surrounding string.
		InterpolationStart(..) | InterpolationEnd(..) => OPERATOR,
		DecimalDigits(..) | HexDigits(..) | OctalDigits(..) | BinaryDigits(..) => NUMBER,
		// `true`/`false` are bool-constructor tokens, not keywords in the
		// strict sense, but render best as keywords.
		BoolTrue(..) | BoolFalse(..) => KEYWORD,
		KeywordAlias(..) | KeywordAs(..) | KeywordDef(..) | KeywordElse(..) | KeywordEnum(..)
		| KeywordFun(..) | KeywordIf(..) | KeywordImplement(..) | KeywordIn(..) | KeywordIs(..)
		| KeywordLet(..) | KeywordTrait(..) | KeywordUse(..) | KeywordWhen(..) | KeywordWhere(..)
		| KeywordWhile(..) => KEYWORD,
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
		| DoubleStar(..)
		| Equal(..)
		| ForwardSlash(..)
		| LeftAngle(..)
		| LeftAngleEqual(..)
		| Minus(..)
		| UnaryMinus(..)
		| Percent(..)
		| Pipe(..)
		| Plus(..)
		| Question(..)
		| RightAngle(..)
		| RightAngleEqual(..)
		| Star(..)
		| Tilde(..)
		| And(..)
		| Backtick(..) => OPERATOR,
		// Identifiers and paths are intentionally left to the AST pass.
		// Pure punctuation (parens, braces, commas, dots, underscore,
		// indent/outdent) is unhighlighted on purpose.
		_ => return None,
	})
}

fn encode_deltas(mut tokens: Vec<Abs>) -> Vec<SemanticToken> {
	tokens.sort_by_key(|t| (t.line, t.col));

	// Collapse exact-position duplicates: a later push for the same
	// (line, col) wins (AST pass appends after lexer, so AST overrides).
	// Also drop entries entirely contained inside the previous one (e.g.
	// a lexer keyword inside an AST-classified region) — LSP rejects
	// overlapping tokens.
	let mut deduped: Vec<Abs> = Vec::with_capacity(tokens.len());
	for tok in tokens {
		while let Some(last) = deduped.last().copied() {
			if last.line == tok.line && last.col == tok.col {
				deduped.pop();
				continue;
			}
			if last.line == tok.line && last.col + last.length > tok.col {
				// Token starts inside the previous one — skip it.
				continue;
			}
			break;
		}
		// Final guard: if we popped past the `while` but still overlap,
		// the candidate falls inside a survivor; just skip it.
		if let Some(last) = deduped.last() {
			if last.line == tok.line && last.col + last.length > tok.col {
				continue;
			}
		}
		deduped.push(tok);
	}

	let mut out = Vec::with_capacity(deduped.len());
	let mut prev_line = 0u32;
	let mut prev_col = 0u32;
	for tok in deduped {
		let delta_line = tok.line - prev_line;
		let delta_start = if delta_line == 0 {
			tok.col - prev_col
		} else {
			tok.col
		};
		out.push(SemanticToken {
			delta_line,
			delta_start,
			length: tok.length,
			token_type: tok.kind,
			token_modifiers_bitset: 0,
		});
		prev_line = tok.line;
		prev_col = tok.col;
	}
	out
}

// -- AST walker -----------------------------------------------------------

struct AstWalker {
	// Names bound by `use` at this module's top level — when seen as a
	// FieldAccess receiver they classify as NAMESPACE.
	module_names: HashSet<String>,
	// Names bound by top-level `enum` defs — when seen as a FieldAccess
	// receiver they classify as ENUM (e.g. `color.red`).
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

	fn walk_module(&mut self, m: &ModuleNode, out: &mut Vec<Abs>) {
		for u in &m.uses {
			self.walk_use(u, out);
		}
		for def in &m.body {
			self.walk_def(def, out);
		}
	}

	fn walk_use(&mut self, u: &UseNode, out: &mut Vec<Abs>) {
		for seg in &u.path {
			emit(out, &seg.range, NAMESPACE, seg.name.len());
		}
		if let Some(alias) = &u.alias {
			emit(out, &alias.range, NAMESPACE, alias.name.len());
		}
	}

	fn walk_def(&mut self, d: &DefinitionNode, out: &mut Vec<Abs>) {
		match &d.kind {
			DefinitionKind::Expr(expr) => {
				let kind = if let ExprKind::Fun(_) = &expr.kind {
					FUNCTION
				} else {
					VARIABLE
				};
				emit(out, &d.name.range, kind, d.name.name.len());
				self.walk_expr(expr, out);
			}
			DefinitionKind::Alias(ty_expr) => {
				emit(out, &d.name.range, TYPE, d.name.name.len());
				self.walk_type_expr(ty_expr, out);
			}
			DefinitionKind::Enum(en) => {
				emit(out, &d.name.range, ENUM, d.name.name.len());
				for p in &en.params {
					emit(out, &p.range, TYPE_PARAMETER, p.name.len());
				}
				for v in &en.variants {
					emit(out, &v.name.range, ENUM_MEMBER, v.name.name.len());
					if let Some(params) = &v.params {
						for p in params {
							self.walk_type_expr(p, out);
						}
					}
				}
			}
			DefinitionKind::Trait(t) => {
				emit(out, &d.name.range, INTERFACE, d.name.name.len());
				emit(out, &t.param.range, TYPE_PARAMETER, t.param.name.len());
				for m in &t.methods {
					emit(out, &m.name.range, FUNCTION, m.name.name.len());
					self.walk_type_expr(&m.signature, out);
					if let Some(default) = &m.default {
						self.walk_expr(default, out);
					}
				}
			}
			DefinitionKind::Instance(inst) => {
				// `d.name` for an instance is a synthetic placeholder; the
				// visible name in source is the trait name.
				emit(
					out,
					&inst.trait_name.range,
					INTERFACE,
					inst.trait_name.name.len(),
				);
				self.walk_type_expr(&inst.head, out);
				for c in &inst.where_clause {
					emit(out, &c.trait_name.range, INTERFACE, c.trait_name.name.len());
					emit(out, &c.param.range, TYPE_PARAMETER, c.param.name.len());
				}
				for method in &inst.methods {
					self.walk_def(method, out);
				}
			}
		}
	}

	fn walk_expr(&mut self, e: &ExprNode, out: &mut Vec<Abs>) {
		match &e.kind {
			ExprKind::Identifier(ident) => {
				let kind = if self.module_names.contains(&ident.name) {
					NAMESPACE
				} else if self.enum_names.contains(&ident.name) {
					ENUM
				} else {
					VARIABLE
				};
				emit(out, &ident.range, kind, ident.name.len());
			}
			ExprKind::Literal(_) => {
				// Lexer pass classified strings/numbers/bools already.
			}
			ExprKind::Regex(r) => {
				emit(out, &r.range, REGEXP, range_len_hint(&r.range));
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
				// Special case: `module.value` or `enum.variant` —
				// classify the receiver based on its bound role and the
				// field by what kind of receiver it is.
				let (receiver_kind, field_kind) = match &receiver.kind {
					ExprKind::Identifier(id) if self.module_names.contains(&id.name) => {
						(Some(NAMESPACE), PROPERTY)
					}
					ExprKind::Identifier(id) if self.enum_names.contains(&id.name) => {
						(Some(ENUM), ENUM_MEMBER)
					}
					_ => (None, PROPERTY),
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
				// Promote a plain Identifier callee to FUNCTION.
				match &c.callee.kind {
					ExprKind::Identifier(id) => {
						emit(out, &id.range, FUNCTION, id.name.len());
					}
					_ => self.walk_expr(&c.callee, out),
				}
				for arg in &c.args {
					self.walk_expr(arg, out);
				}
			}
			ExprKind::EmptyTuple => {}
			ExprKind::Grouping(inner) => self.walk_expr(inner, out),
			ExprKind::Interpolation(parts) => {
				for p in parts {
					self.walk_expr(p, out);
				}
			}
			ExprKind::Let(l) => {
				match &l.pattern.kind {
					PatternKind::Identifier(id) => {
						let kind = if let ExprKind::Fun(_) = &l.value.kind {
							FUNCTION
						} else {
							VARIABLE
						};
						emit(out, &id.range, kind, id.name.len());
					}
					_ => self.walk_pattern(&l.pattern, out),
				}
				self.walk_expr(&l.value, out);
			}
			ExprKind::Record(fields) => {
				for (name, value) in fields {
					emit(out, &name.range, PROPERTY, name.name.len());
					self.walk_expr(value, out);
				}
			}
			ExprKind::Tuple(elements) | ExprKind::List(elements) => {
				for el in elements {
					self.walk_expr(el, out);
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
			ExprKind::NamespaceAccess(_) => {
				// semantic-tokens runs on parser output (see collect_semantic_tokens),
				// which only produces FieldAccess; NamespaceAccess is created later
				// by the analyzer.
			}
		}
	}

	fn walk_fun(&mut self, f: &FunNode, out: &mut Vec<Abs>) {
		for p in &f.params {
			emit(out, &p.ident.range, PARAMETER, p.ident.name.len());
		}
		for e in &f.body {
			self.walk_expr(e, out);
		}
	}

	fn walk_pattern(&mut self, p: &PatternNode, out: &mut Vec<Abs>) {
		match &p.kind {
			PatternKind::Identifier(id) => {
				emit(out, &id.range, VARIABLE, id.name.len());
			}
			PatternKind::Constructor(name, inner) => {
				emit(out, &name.range, ENUM_MEMBER, name.name.len());
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
					emit(out, &name.range, PROPERTY, name.name.len());
					self.walk_pattern(sub, out);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						emit(out, &name.range, VARIABLE, name.name.len());
					}
				}
			}
			PatternKind::List { items, rest } => {
				for ip in items {
					self.walk_pattern(ip, out);
				}
				if let Some(rp) = rest {
					if let Some(name) = &rp.binding {
						emit(out, &name.range, VARIABLE, name.name.len());
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

	fn walk_type_expr(&mut self, t: &TypeExprNode, out: &mut Vec<Abs>) {
		match &t.kind {
			TypeExprKind::Single(id) => {
				// `id.range` covers the whole `module.TypeName` span — emit
				// the TYPE token at the name's actual position. With a
				// module prefix the name starts right after the dot; bare
				// types start at range.start.
				let name_line = id.range.start.line as u32;
				let name_col = match &id.module {
					Some(module) => {
						emit(out, &module.range, NAMESPACE, module.name.len());
						(module.range.end.col + 1) as u32
					}
					None => id.range.start.col as u32,
				};
				out.push(Abs {
					line: name_line,
					col: name_col,
					length: id.name.len() as u32,
					kind: TYPE,
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
					emit(out, &name.range, PROPERTY, name.name.len());
					self.walk_type_expr(ty, out);
				}
			}
			TypeExprKind::EmptyTuple => {}
			TypeExprKind::Grouping(inner) => self.walk_type_expr(inner, out),
		}
	}
}

fn emit(out: &mut Vec<Abs>, range: &compiler::Range, kind: u32, length: usize) {
	// Skip multi-line ranges — LSP semantic tokens must be single-line.
	if range.start.line != range.end.line {
		return;
	}
	out.push(Abs {
		line: range.start.line as u32,
		col: range.start.col as u32,
		length: length as u32,
		kind,
	});
}

fn range_len_hint(range: &compiler::Range) -> usize {
	if range.start.line == range.end.line {
		range.end.col.saturating_sub(range.start.col)
	} else {
		0
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Decode the LSP delta-encoded stream back into absolute
	// (line, col, length, kind) tuples for easier assertion.
	fn decode(tokens: &[SemanticToken]) -> Vec<(u32, u32, u32, u32)> {
		let mut out = Vec::new();
		let mut line = 0u32;
		let mut col = 0u32;
		for t in tokens {
			if t.delta_line == 0 {
				col += t.delta_start;
			} else {
				line += t.delta_line;
				col = t.delta_start;
			}
			out.push((line, col, t.length, t.token_type));
		}
		out
	}

	fn classify(src: &str) -> Vec<(u32, u32, u32, u32)> {
		decode(&collect_semantic_tokens(&src.as_bytes().to_vec()))
	}

	#[test]
	fn keywords_and_literals() {
		// `let` is an expression form; wrap it in a def so it parses.
		let src = "def main = fun {\n\tlet x = 42\n}\n";
		let toks = classify(src);
		assert!(toks.contains(&(1, 1, 3, KEYWORD)), "let keyword");
		assert!(toks.contains(&(1, 5, 1, VARIABLE)), "x binding");
		assert!(toks.contains(&(1, 7, 1, OPERATOR)), "= operator");
		assert!(toks.contains(&(1, 9, 2, NUMBER)), "42 literal");
	}

	#[test]
	fn def_with_fun_is_function() {
		let toks = classify("def greet = fun name {\n\tprint name\n}\n");
		assert!(toks.contains(&(0, 0, 3, KEYWORD)), "def");
		assert!(toks.contains(&(0, 4, 5, FUNCTION)), "greet is function");
		assert!(toks.contains(&(0, 12, 3, KEYWORD)), "fun");
		assert!(toks.contains(&(0, 16, 4, PARAMETER)), "name param");
		assert!(toks.contains(&(1, 1, 5, FUNCTION)), "print call");
		assert!(toks.contains(&(1, 7, 4, VARIABLE)), "name use");
	}

	#[test]
	fn enum_def_and_variant_access() {
		let src = "enum color {\n\tred\n\tgreen\n}\ndef c = color.red\n";
		let toks = classify(src);
		assert!(toks.contains(&(0, 0, 4, KEYWORD)), "enum kw");
		assert!(toks.contains(&(0, 5, 5, ENUM)), "color name");
		assert!(toks.contains(&(1, 1, 3, ENUM_MEMBER)), "red variant");
		assert!(toks.contains(&(4, 8, 5, ENUM)), "color receiver");
		assert!(toks.contains(&(4, 14, 3, ENUM_MEMBER)), "red access");
	}

	#[test]
	fn use_and_qualified_call() {
		let src = "use math\n\ndef x = math.add 1 2\n";
		let toks = classify(src);
		assert!(toks.contains(&(0, 0, 3, KEYWORD)), "use kw");
		assert!(toks.contains(&(0, 4, 4, NAMESPACE)), "math import");
		assert!(toks.contains(&(2, 8, 4, NAMESPACE)), "math receiver");
		assert!(toks.contains(&(2, 13, 3, PROPERTY)), "add field");
	}

	#[test]
	fn alias_record_with_typed_fields() {
		let src = "alias person {\n\tname :: string\n\tage  :: int\n}\n";
		let toks = classify(src);
		assert!(toks.contains(&(0, 0, 5, KEYWORD)), "alias kw");
		assert!(toks.contains(&(0, 6, 6, TYPE)), "person type");
		assert!(toks.contains(&(1, 1, 4, PROPERTY)), "name field");
		assert!(toks.contains(&(1, 9, 6, TYPE)), "string type");
		assert!(toks.contains(&(2, 1, 3, PROPERTY)), "age field");
		assert!(toks.contains(&(2, 9, 3, TYPE)), "int type");
	}

	#[test]
	fn strings_cover_content() {
		// Tokenizer's StringLiteral span covers content between quotes,
		// not the quotes themselves. TextMate handles the surrounding
		// quotes — the semantic-token overlay just needs to mark the
		// content's role so it composes well visually.
		let src = "def s = \"hello\"\n";
		let toks = classify(src);
		assert!(
			toks.contains(&(0, 9, 5, STRING)),
			"hello content: {:?}",
			toks
		);
	}
}
