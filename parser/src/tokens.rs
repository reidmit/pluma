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

	/// `\` token
	BackSlash(usize, usize),

	/// `!` token
	Bang(usize, usize),

	/// `!=` token
	BangEqual(usize, usize),

	/// e.g. `0b10101`
	BinaryDigits(usize, usize),

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

	/// `.` token
	Dot(usize, usize),

	/// `&&` token
	DoubleAnd(usize, usize),

	/// `=>` token
	DoubleArrow(usize, usize),

	/// `::` token
	DoubleColon(usize, usize),

	/// `==` token
	DoubleEqual(usize, usize),

	/// `<<` token
	DoubleLeftAngle(usize, usize),

	/// `||` token
	DoublePipe(usize, usize),

	/// `++ token
	DoublePlus(usize, usize),

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

	/// e.g. `path/to/some/module`
	ImportPath(usize, usize),

	/// e.g. `)` in `"hello $(name)"`
	InterpolationEnd(usize, usize),

	/// e.g. `$(` in `"hello $(name)"`
	InterpolationStart(usize, usize),

	/// `alias` keyword
	KeywordAlias(usize, usize),

	/// `case` keyword
	KeywordCase(usize, usize),

	/// `def` keyword
	KeywordDef(usize, usize),

	/// `enum` keyword
	KeywordEnum(usize, usize),

	/// `let` keyword
	KeywordLet(usize, usize),

	/// `match` keyword
	KeywordMatch(usize, usize),

	/// `mut` keyword
	KeywordMut(usize, usize),

	/// `struct` keyword
	KeywordStruct(usize, usize),

	/// `trait` keyword
	KeywordTrait(usize, usize),

	/// `use` keyword
	KeywordUse(usize, usize),

	/// `where` keyword
	KeywordWhere(usize, usize),

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

	/// `-` token
	Minus(usize, usize),

	/// e.g. `0o755`
	OctalDigits(usize, usize),

	/// `%` token
	Percent(usize, usize),

	/// `|` token
	Pipe(usize, usize),

	/// `+` token
	Plus(usize, usize),

	/// e.g. `@pkg` in `@pkg some-identifier`
	Qualifier(usize, usize),

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

	/// `~` token
	Tilde(usize, usize),

	/// `_` token
	Underscore(usize, usize),

	/// any unexpected token
	Unexpected(usize, usize),
}

impl Token {
	pub fn get_position(&self) -> (usize, usize) {
		use Token::*;

		match self {
			And(start, end)
			| Arrow(start, end)
			| BackSlash(start, end)
			| Bang(start, end)
			| BangEqual(start, end)
			| BinaryDigits(start, end)
			| Caret(start, end)
			| Colon(start, end)
			| Comma(start, end)
			| Comment(start, end)
			| DecimalDigits(start, end)
			| Dot(start, end)
			| DoubleAnd(start, end)
			| DoubleArrow(start, end)
			| DoubleColon(start, end)
			| DoubleEqual(start, end)
			| DoubleLeftAngle(start, end)
			| DoublePipe(start, end)
			| DoublePlus(start, end)
			| DoubleRightAngle(start, end)
			| DoubleStar(start, end)
			| Equal(start, end)
			| ForwardSlash(start, end)
			| HexDigits(start, end)
			| Identifier(start, end)
			| ImportPath(start, end)
			| InterpolationEnd(start, end)
			| InterpolationStart(start, end)
			| KeywordAlias(start, end)
			| KeywordCase(start, end)
			| KeywordDef(start, end)
			| KeywordEnum(start, end)
			| KeywordLet(start, end)
			| KeywordMatch(start, end)
			| KeywordMut(start, end)
			| KeywordStruct(start, end)
			| KeywordTrait(start, end)
			| KeywordUse(start, end)
			| KeywordWhere(start, end)
			| LeftAngle(start, end)
			| LeftAngleEqual(start, end)
			| LeftBrace(start, end)
			| LeftBracket(start, end)
			| LeftParen(start, end)
			| LineBreak(start, end)
			| Minus(start, end)
			| OctalDigits(start, end)
			| Percent(start, end)
			| Pipe(start, end)
			| Plus(start, end)
			| Qualifier(start, end)
			| Question(start, end)
			| RightAngle(start, end)
			| RightAngleEqual(start, end)
			| RightBrace(start, end)
			| RightBracket(start, end)
			| RightParen(start, end)
			| Star(start, end)
			| StringLiteral(start, end)
			| Tilde(start, end)
			| Underscore(start, end)
			| Unexpected(start, end) => (*start, *end),
		}
	}

	pub fn can_start_expression(&self) -> bool {
		use Token::*;

		match self {
			Identifier(..) | BackSlash(..) | Colon(..) | DecimalDigits(..) | HexDigits(..)
			| BinaryDigits(..) | OctalDigits(..) | LeftParen(..) | LeftBrace(..) | LeftBracket(..)
			| ForwardSlash(..) | StringLiteral(..) => true,
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
			&BackSlash(..) => "a '\\'",
			&Bang(..) => "a '!'",
			&BangEqual(..) => "a '!='",
			&BinaryDigits(..) => "binary digits (e.g. 0b101)",
			&Caret(..) => "a '^'",
			&Colon(..) => "a ':'",
			&Comma(..) => "a ','",
			&Comment(..) => "a comment",
			&DecimalDigits(..) => "decimal digits (e.g. 47)",
			&Dot(..) => "a '.'",
			&DoubleAnd(..) => "a '&&'",
			&DoubleArrow(..) => "a '=>'",
			&DoubleColon(..) => "a '::'",
			&DoubleEqual(..) => "a '=='",
			&DoubleLeftAngle(..) => "a '<<'",
			&DoublePipe(..) => "a '||'",
			&DoublePlus(..) => "a '++'",
			&DoubleRightAngle(..) => "a '>>'",
			&DoubleStar(..) => "a '||'",
			&Equal(..) => "a '='",
			&ForwardSlash(..) => "a '/'",
			&HexDigits(..) => "hex digits (e.g. 0xf4c3)",
			&Identifier(..) => "an identifier",
			&ImportPath(..) => "a path to a module (e.g. 'path/to/module')",
			&InterpolationEnd(..) => "a ')'",
			&InterpolationStart(..) => "a '$('",
			&KeywordAlias(..) => "keyword 'alias'",
			&KeywordCase(..) => "keyword 'case'",
			&KeywordDef(..) => "keyword 'def'",
			&KeywordEnum(..) => "keyword 'enum'",
			&KeywordLet(..) => "keyword 'let'",
			&KeywordMatch(..) => "keyword 'match'",
			&KeywordMut(..) => "keyword 'mut'",
			&KeywordStruct(..) => "keyword 'struct'",
			&KeywordTrait(..) => "keyword 'trait'",
			&KeywordUse(..) => "keyword 'use'",
			&KeywordWhere(..) => "keyword 'where'",
			&LeftAngle(..) => "a '<'",
			&LeftAngleEqual(..) => "a '<='",
			&LeftBrace(..) => "a '{'",
			&LeftBracket(..) => "a '['",
			&LeftParen(..) => "a '('",
			&LineBreak(..) => "a line break",
			&Minus(..) => "a '-'",
			&OctalDigits(..) => "octal digits (e.g. 0o755)",
			&Percent(..) => "a '%'",
			&Pipe(..) => "a '|'",
			&Plus(..) => "a '+'",
			&Qualifier(..) => "a qualifier",
			&Question(..) => "a '?'",
			&RightAngle(..) => "a '>'",
			&RightAngleEqual(..) => "a '>='",
			&RightBrace(..) => "a '}'",
			&RightBracket(..) => "a ']'",
			&RightParen(..) => "a ')'",
			&Star(..) => "a '*'",
			&StringLiteral(..) => "a string",
			&Tilde(..) => "a '~'",
			&Underscore(..) => "a '_'",
			&Unexpected(..) => "unknown",
		};

		write!(f, "{}", as_string)
	}
}
