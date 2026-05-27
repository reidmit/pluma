use std::fmt;

/// Represents a token in the source code.
///
/// Each token has a start and end index associated with it (byte index into
/// the source code file).
#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Token {
	/// `&` token
	And(usize, usize),

	/// `->` token
	Arrow(usize, usize),

	/// `\`` token
	Backtick(usize, usize),

	/// `!` token
	Bang(usize, usize),

	/// `!=` token
	BangEqual(usize, usize),

	/// e.g. `0b10101`
	BinaryDigits(usize, usize),

	/// `true` (bool constructor)
	BoolTrue(usize, usize),

	/// `false` (bool constructor)
	BoolFalse(usize, usize),

	/// `^` token
	Caret(usize, usize),

	/// `:` token
	Colon(usize, usize),

	/// `,` token
	Comma(usize, usize),

	/// e.g. # hello ... (until end of line)
	Comment(usize, usize),

	/// e.g. `47`
	DecimalDigits(usize, usize),

	/// e.g. `5s`, `2m20s`, `3h2m10s` — digits immediately followed by a time
	/// unit. The span covers the whole run; the parser splits it into
	/// `<amount><unit>` segments and validates order/range.
	DurationLiteral(usize, usize),

	/// `.` token
	Dot(usize, usize),

	/// `$` token (only emitted when not followed by `(`; `$(...)` opens an
	/// `InterpolationStart` instead)
	Dollar(usize, usize),

	/// `&&` token
	DoubleAnd(usize, usize),

	/// `::` token
	DoubleColon(usize, usize),

	/// `..` token
	DoubleDot(usize, usize),

	/// `...` token (rest pattern in list patterns)
	TripleDot(usize, usize),

	/// `==` token
	DoubleEqual(usize, usize),

	/// `//` token
	DoubleForwardSlash(usize, usize),

	/// `<<` token
	DoubleLeftAngle(usize, usize),

	/// `||` token
	DoublePipe(usize, usize),

	/// `++ token
	DoublePlus(usize, usize),

	/// `??` token
	DoubleQuestion(usize, usize),

	/// `>>` token
	DoubleRightAngle(usize, usize),

	/// `**` token
	DoubleStar(usize, usize),

	/// `=` token
	Equal(usize, usize),

	/// `/` token
	ForwardSlash(usize, usize),

	/// e.g. `0xbeef`
	HexDigits(usize, usize),

	/// e.g. `hello` or `hello-world`
	Identifier(usize, usize),

	/// an increase in indentation level
	Indent(usize, usize),

	/// e.g. `}` in `"hello ${name}"`
	InterpolationEnd(usize, usize),

	/// e.g. `${` in `"hello ${name}"`
	InterpolationStart(usize, usize),

	/// `alias` keyword
	KeywordAlias(usize, usize),

	/// `as` keyword (import alias)
	KeywordAs(usize, usize),

	/// `built-in` keyword. Marker for the RHS of a top-level def whose
	/// implementation lives in Rust; the string literal that follows
	/// names the entry in the tag table (`built-in "list-length"`).
	KeywordBuiltin(usize, usize),

	/// `def` keyword
	KeywordDef(usize, usize),

	/// `else` keyword (catch-all branch on `if` / `when`)
	KeywordElse(usize, usize),

	/// `enum` keyword
	KeywordEnum(usize, usize),

	/// `fun` keyword
	KeywordFun(usize, usize),

	/// `implement` keyword (instance declaration: `implement TRAIT TYPE { ... }`)
	KeywordImplement(usize, usize),

	/// `if` keyword
	KeywordIf(usize, usize),

	/// `is` keyword
	KeywordIs(usize, usize),

	/// `in` keyword
	KeywordIn(usize, usize),

	/// `defer` keyword (schedule cleanup at enclosing-function exit)
	KeywordDefer(usize, usize),

	/// `let` keyword
	KeywordLet(usize, usize),

	/// `scope` keyword (structured-concurrency block)
	KeywordScope(usize, usize),

	/// `manual` keyword — the prefix modifier for `manual scope` (the
	/// non-fail-fast scope form). A reserved word; only meaningful directly
	/// before `scope`.
	KeywordManual(usize, usize),

	/// `opaque` keyword (visibility: export an enum's type but hide its
	/// constructors)
	KeywordOpaque(usize, usize),

	/// `public` keyword (visibility: export a top-level def fully)
	KeywordPublic(usize, usize),

	/// `trait` keyword
	KeywordTrait(usize, usize),

	/// `try` keyword (sequential-bind form: `try x = expr ; rest`).
	/// Dispatches at analyze time based on the inferred head constructor
	/// of `expr` — option, result, or task — and rewrites to a
	/// `<carrier>.then` call wrapping the remaining block items.
	KeywordTry(usize, usize),

	/// `use` keyword (module import)
	KeywordUse(usize, usize),

	/// `when` keyword
	KeywordWhen(usize, usize),

	/// `where` keyword (instance constraints)
	KeywordWhere(usize, usize),

	/// `while` keyword
	KeywordWhile(usize, usize),

	/// `<` token
	LeftAngle(usize, usize),

	/// `<=` token
	LeftAngleEqual(usize, usize),

	/// `{` token
	LeftBrace(usize, usize),

	/// `[` token
	LeftBracket(usize, usize),

	/// `(` token
	LeftParen(usize, usize),

	/// a newline
	LineBreak(usize, usize),

	/// a decrease in indentation level
	LineBreakWithIndentDecrease(usize, usize),

	/// an increase in indentation level
	LineBreakWithIndentIncrease(usize, usize),

	/// `-` token in binary or unambiguous-prefix position (e.g. `a - b`,
	/// `a-b`, `(-x)`). Acts as either infix subtract or prefix negate.
	Minus(usize, usize),

	/// `-` token in `[whitespace]-[non-whitespace]` position (e.g. `f -x`).
	/// Only valid as prefix negate, never as infix subtraction — lets
	/// `f -x` parse as `f(-x)` instead of `f - x`.
	UnaryMinus(usize, usize),

	/// e.g. `0o755`
	OctalDigits(usize, usize),

	/// a decrease in indentation level
	Outdent(usize, usize),

	/// e.g. `path/to/some/module`
	Path(usize, usize),

	/// `%` token
	Percent(usize, usize),

	/// `|` token
	Pipe(usize, usize),

	/// `+` token
	Plus(usize, usize),

	/// `?` token (appears in reg exprs)
	Question(usize, usize),

	/// `>` token
	RightAngle(usize, usize),

	/// `>=` token
	RightAngleEqual(usize, usize),

	/// `}` token
	RightBrace(usize, usize),

	/// `]` token
	RightBracket(usize, usize),

	/// `)` token
	RightParen(usize, usize),

	/// `*` token
	Star(usize, usize),

	/// e.g. `"hello"`
	StringLiteral(usize, usize),

	/// e.g. `'\x89PNG'`. Span covers the inner bytes only, like StringLiteral.
	BytesLiteral(usize, usize),

	/// `~` token
	Tilde(usize, usize),

	/// `_` token
	Underscore(usize, usize),

	/// any unexpected token
	Unexpected(u8, usize, usize),
}

impl Token {
	pub fn length(&self) -> usize {
		let (start, end) = self.get_span();
		end - start
	}

	pub fn is_line_break(&self) -> bool {
		match self {
			Token::LineBreak(..)
			| Token::LineBreakWithIndentIncrease(..)
			| Token::LineBreakWithIndentDecrease(..) => true,
			_ => false,
		}
	}

	pub fn get_span(&self) -> (usize, usize) {
		use Token::*;

		match self {
			And(start, end)
			| Arrow(start, end)
			| Backtick(start, end)
			| Bang(start, end)
			| BangEqual(start, end)
			| BinaryDigits(start, end)
			| BoolTrue(start, end)
			| BoolFalse(start, end)
			| Caret(start, end)
			| Colon(start, end)
			| Comma(start, end)
			| Comment(start, end)
			| DecimalDigits(start, end)
			| DurationLiteral(start, end)
			| Dollar(start, end)
			| Dot(start, end)
			| DoubleAnd(start, end)
			| DoubleColon(start, end)
			| DoubleDot(start, end)
			| TripleDot(start, end)
			| DoubleEqual(start, end)
			| DoubleForwardSlash(start, end)
			| DoubleLeftAngle(start, end)
			| DoublePipe(start, end)
			| DoublePlus(start, end)
			| DoubleQuestion(start, end)
			| DoubleRightAngle(start, end)
			| DoubleStar(start, end)
			| Equal(start, end)
			| ForwardSlash(start, end)
			| HexDigits(start, end)
			| Identifier(start, end)
			| Indent(start, end)
			| InterpolationEnd(start, end)
			| InterpolationStart(start, end)
			| KeywordAlias(start, end)
			| KeywordAs(start, end)
			| KeywordBuiltin(start, end)
			| KeywordDef(start, end)
			| KeywordDefer(start, end)
			| KeywordElse(start, end)
			| KeywordEnum(start, end)
			| KeywordFun(start, end)
			| KeywordIf(start, end)
			| KeywordImplement(start, end)
			| KeywordIn(start, end)
			| KeywordIs(start, end)
			| KeywordLet(start, end)
			| KeywordScope(start, end)
			| KeywordManual(start, end)
			| KeywordOpaque(start, end)
			| KeywordPublic(start, end)
			| KeywordTrait(start, end)
			| KeywordTry(start, end)
			| KeywordUse(start, end)
			| KeywordWhile(start, end)
			| KeywordWhen(start, end)
			| KeywordWhere(start, end)
			| LeftAngle(start, end)
			| LeftAngleEqual(start, end)
			| LeftBrace(start, end)
			| LeftBracket(start, end)
			| LeftParen(start, end)
			| LineBreak(start, end)
			| LineBreakWithIndentDecrease(start, end)
			| LineBreakWithIndentIncrease(start, end)
			| Minus(start, end)
			| UnaryMinus(start, end)
			| OctalDigits(start, end)
			| Outdent(start, end)
			| Path(start, end)
			| Percent(start, end)
			| Pipe(start, end)
			| Plus(start, end)
			| Question(start, end)
			| RightAngle(start, end)
			| RightAngleEqual(start, end)
			| RightBrace(start, end)
			| RightBracket(start, end)
			| RightParen(start, end)
			| Star(start, end)
			| StringLiteral(start, end)
			| BytesLiteral(start, end)
			| Tilde(start, end)
			| Underscore(start, end)
			| Unexpected(_, start, end) => (*start, *end),
		}
	}

	pub fn can_start_expression(&self) -> bool {
		use Token::*;

		match self {
			Identifier(..) | KeywordBuiltin(..) | KeywordFun(..) | KeywordIf(..) | KeywordWhen(..)
			| KeywordScope(..) | KeywordManual(..) | DecimalDigits(..) | DurationLiteral(..)
			| HexDigits(..) | BinaryDigits(..) | OctalDigits(..) | LeftParen(..) | LeftBracket(..)
			| LeftBrace(..) | Backtick(..) | StringLiteral(..) | BytesLiteral(..) | BoolTrue(..)
			| BoolFalse(..) | UnaryMinus(..) => true,
			_ => false,
		}
	}
}

impl fmt::Display for Token {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		use Token::*;

		let as_string = match self {
			&And(..) => "a '&'",
			&Arrow(..) => "a '->'",
			&Backtick(..) => "a '`'",
			&Bang(..) => "a '!'",
			&BangEqual(..) => "a '!='",
			&BinaryDigits(..) => "binary digits (e.g. 0b101)",
			&BoolFalse(..) => "`false`",
			&BoolTrue(..) => "`true`",
			&Caret(..) => "a '^'",
			&Colon(..) => "a ':'",
			&Comma(..) => "a ','",
			&Comment(..) => "a comment",
			&DecimalDigits(..) => "decimal digits (e.g. 47)",
			&DurationLiteral(..) => "a duration literal (e.g. 2m20s)",
			&Dollar(..) => "a '$'",
			&Dot(..) => "a '.'",
			&DoubleAnd(..) => "a '&&'",
			&DoubleDot(..) => "a '..'",
			&TripleDot(..) => "a '...'",
			&DoubleColon(..) => "a '::'",
			&DoubleEqual(..) => "a '=='",
			&DoubleForwardSlash(..) => "a '//'",
			&DoubleLeftAngle(..) => "a '<<'",
			&DoublePipe(..) => "a '||'",
			&DoublePlus(..) => "a '++'",
			&DoubleQuestion(..) => "a '??'",
			&DoubleRightAngle(..) => "a '>>'",
			&DoubleStar(..) => "a '**'",
			&Equal(..) => "a '='",
			&ForwardSlash(..) => "a '/'",
			&HexDigits(..) => "hex digits (e.g. 0xf4c3)",
			&Identifier(..) => "an identifier",
			&Indent(..) => "an indent",
			&InterpolationEnd(..) => "a ')'",
			&InterpolationStart(..) => "a '$('",
			&KeywordAlias(..) => "keyword `alias`",
			&KeywordAs(..) => "keyword `as`",
			&KeywordBuiltin(..) => "keyword `built-in`",
			&KeywordDef(..) => "keyword `def`",
			&KeywordDefer(..) => "keyword `defer`",
			&KeywordElse(..) => "keyword `else`",
			&KeywordEnum(..) => "keyword `enum`",
			&KeywordFun(..) => "keyword `fun`",
			&KeywordIf(..) => "keyword `if`",
			&KeywordImplement(..) => "keyword `implement`",
			&KeywordIn(..) => "keyword `in`",
			&KeywordIs(..) => "keyword `is`",
			&KeywordLet(..) => "keyword `let`",
			&KeywordScope(..) => "keyword `scope`",
			&KeywordManual(..) => "keyword `manual`",
			&KeywordOpaque(..) => "keyword `opaque`",
			&KeywordPublic(..) => "keyword `public`",
			&KeywordTrait(..) => "keyword `trait`",
			&KeywordTry(..) => "keyword `try`",
			&KeywordUse(..) => "keyword `use`",
			&KeywordWhile(..) => "keyword `while`",
			&KeywordWhen(..) => "keyword `when`",
			&KeywordWhere(..) => "keyword `where`",
			&LeftAngle(..) => "a '<'",
			&LeftAngleEqual(..) => "a '<='",
			&LeftBrace(..) => "a '{'",
			&LeftBracket(..) => "a '['",
			&LeftParen(..) => "a '('",
			&LineBreak(..) => "a line break",
			&LineBreakWithIndentDecrease(..) => "a decrease in indent level",
			&LineBreakWithIndentIncrease(..) => "an increase in indent level",
			&Minus(..) => "a '-'",
			&UnaryMinus(..) => "a '-' (prefix)",
			&OctalDigits(..) => "octal digits (e.g. 0o755)",
			&Outdent(..) => "an outdent",
			&Path(..) => "a path to a module or imported identifier (e.g. 'path/to/module')",
			&Percent(..) => "a '%'",
			&Pipe(..) => "a '|'",
			&Plus(..) => "a '+'",
			&Question(..) => "a '?'",
			&RightAngle(..) => "a '>'",
			&RightAngleEqual(..) => "a '>='",
			&RightBrace(..) => "a '}'",
			&RightBracket(..) => "a ']'",
			&RightParen(..) => "a ')'",
			&Star(..) => "a '*'",
			&StringLiteral(..) => "a string",
			&BytesLiteral(..) => "a bytes literal",
			&Tilde(..) => "a '~'",
			&Underscore(..) => "a '_'",
			&Unexpected(c, ..) => return write!(f, "'{}'", String::from_utf8_lossy(&[c])),
		};

		write!(f, "{}", as_string)
	}
}
