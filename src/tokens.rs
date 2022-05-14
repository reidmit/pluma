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

	/// `..` token
	DoubleDot(usize, usize),

	/// `==` token
	DoubleEqual(usize, usize),

	/// `/` token
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

	/// e.g. `}` in `"hello ${name}"`
	InterpolationEnd(usize, usize),

	/// e.g. `${` in `"hello ${name}"`
	InterpolationStart(usize, usize),

	/// `alias` keyword
	KeywordAlias(usize, usize),

	/// `def` keyword
	KeywordDef(usize, usize),

	/// `enum` keyword
	KeywordEnum(usize, usize),

	/// `for` keyword
	KeywordFor(usize, usize),

	/// `fun` keyword
	KeywordFun(usize, usize),

	/// `if` keyword
	KeywordIf(usize, usize),

	/// `is` keyword
	KeywordIs(usize, usize),

	/// `in` keyword
	KeywordIn(usize, usize),

	/// `let` keyword
	KeywordLet(usize, usize),

	/// `struct` keyword
	KeywordStruct(usize, usize),

	/// `trait` keyword
	KeywordTrait(usize, usize),

	/// `use` keyword
	KeywordUse(usize, usize),

	/// `when` keyword
	KeywordWhen(usize, usize),

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

	/// `-` token
	Minus(usize, usize),

	/// e.g. `0o755`
	OctalDigits(usize, usize),

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

	/// `;` token
	Semicolon(usize, usize),

	/// `*` token
	Star(usize, usize),

	/// e.g. `"hello"`
	StringLiteral(usize, usize),

	/// `~` token
	Tilde(usize, usize),

	/// `_` token
	Underscore(usize, usize),

	/// any unexpected token
	Unexpected(u8, usize, usize),
}

impl Token {
	pub fn get_position(&self) -> (usize, usize) {
		use Token::*;

		match self {
			And(start, end)
			| Arrow(start, end)
			| Backtick(start, end)
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
			| DoubleDot(start, end)
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
			| InterpolationEnd(start, end)
			| InterpolationStart(start, end)
			| KeywordAlias(start, end)
			| KeywordDef(start, end)
			| KeywordEnum(start, end)
			| KeywordFor(start, end)
			| KeywordFun(start, end)
			| KeywordIf(start, end)
			| KeywordIn(start, end)
			| KeywordIs(start, end)
			| KeywordLet(start, end)
			| KeywordStruct(start, end)
			| KeywordTrait(start, end)
			| KeywordUse(start, end)
			| KeywordWhen(start, end)
			| KeywordWhile(start, end)
			| LeftAngle(start, end)
			| LeftAngleEqual(start, end)
			| LeftBrace(start, end)
			| LeftBracket(start, end)
			| LeftParen(start, end)
			| LineBreak(start, end)
			| Minus(start, end)
			| OctalDigits(start, end)
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
			| Semicolon(start, end)
			| Star(start, end)
			| StringLiteral(start, end)
			| Tilde(start, end)
			| Underscore(start, end)
			| Unexpected(_, start, end) => (*start, *end),
		}
	}

	pub fn can_start_expression(&self) -> bool {
		use Token::*;

		match self {
			Identifier(..) | KeywordFun(..) | KeywordIf(..) | DecimalDigits(..) | HexDigits(..)
			| BinaryDigits(..) | OctalDigits(..) | LeftParen(..) | LeftBrace(..) | LeftBracket(..)
			| Backtick(..) | StringLiteral(..) => true,
			_ => false,
		}
	}
}

impl fmt::Display for Token {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		use Token::*;

		if let Unexpected(c, ..) = self {
			return write!(f, "'{}'", String::from_utf8_lossy(&[*c]));
		}

		let as_string = match self {
			&And(..) => "a '&'",
			&Arrow(..) => "a '->'",
			&Backtick(..) => "a '`'",
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
			&DoubleDot(..) => "a '..'",
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
			&InterpolationEnd(..) => "a ')'",
			&InterpolationStart(..) => "a '$('",
			&KeywordAlias(..) => "keyword 'alias'",
			&KeywordDef(..) => "keyword 'def'",
			&KeywordEnum(..) => "keyword 'enum'",
			&KeywordFor(..) => "keyword 'for'",
			&KeywordFun(..) => "keyword 'fun'",
			&KeywordIf(..) => "keyword 'if'",
			&KeywordIn(..) => "keyword 'in'",
			&KeywordIs(..) => "keyword 'is'",
			&KeywordLet(..) => "keyword 'let'",
			&KeywordStruct(..) => "keyword 'struct'",
			&KeywordTrait(..) => "keyword 'trait'",
			&KeywordUse(..) => "keyword 'use'",
			&KeywordWhen(..) => "keyword 'when'",
			&KeywordWhile(..) => "keyword 'while'",
			&LeftAngle(..) => "a '<'",
			&LeftAngleEqual(..) => "a '<='",
			&LeftBrace(..) => "a '{'",
			&LeftBracket(..) => "a '['",
			&LeftParen(..) => "a '('",
			&LineBreak(..) => "a line break",
			&Minus(..) => "a '-'",
			&OctalDigits(..) => "octal digits (e.g. 0o755)",
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
			&Semicolon(..) => "a ';'",
			&Star(..) => "a '*'",
			&StringLiteral(..) => "a string",
			&Tilde(..) => "a '~'",
			&Underscore(..) => "a '_'",
			&Unexpected(..) => unreachable!("handled above"),
		};

		write!(f, "{}", as_string)
	}
}
