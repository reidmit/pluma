//! Semantic tokens for the editor — a thin adapter over the shared
//! `compiler::highlight` classifier (the same one that highlights the docs
//! site). We map each highlight `Class` to this server's token-type legend and
//! delta-encode; all the actual classification lives in the compiler.

use compiler::highlight::{self, Class};
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

// The legend index for a highlight class.
fn token_type(class: Class) -> u32 {
	match class {
		Class::Comment => COMMENT,
		Class::Function => FUNCTION,
		Class::Variable => VARIABLE,
		Class::String => STRING,
		Class::Number => NUMBER,
		Class::Regexp => REGEXP,
		Class::Keyword => KEYWORD,
		Class::Operator => OPERATOR,
		Class::Type => TYPE,
		Class::Parameter => PARAMETER,
		Class::Property => PROPERTY,
		Class::Enum => ENUM,
		Class::EnumMember => ENUM_MEMBER,
		Class::Namespace => NAMESPACE,
		Class::Interface => INTERFACE,
		Class::TypeParameter => TYPE_PARAMETER,
	}
}

pub fn collect_semantic_tokens(source: &Vec<u8>) -> Vec<SemanticToken> {
	// `classify` returns tokens sorted by position, non-overlapping, single-line
	// — exactly what the LSP delta encoding needs.
	let tokens = highlight::classify(source);

	let mut out = Vec::with_capacity(tokens.len());
	let mut prev_line = 0u32;
	let mut prev_col = 0u32;
	for tok in tokens {
		let line = tok.line as u32;
		let col = tok.col as u32;
		let delta_line = line - prev_line;
		let delta_start = if delta_line == 0 { col - prev_col } else { col };
		out.push(SemanticToken {
			delta_line,
			delta_start,
			length: tok.len as u32,
			token_type: token_type(tok.class),
			token_modifiers_bitset: 0,
		});
		prev_line = line;
		prev_col = col;
	}
	out
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
	fn strings_include_quotes() {
		// The tokenizer's StringLiteral span covers only the content between
		// the quotes; the lexer pass extends it to include the delimiting `"`
		// so grammar-less editors (Zed) highlight the quotes too. `"hello"`
		// starts at col 8 and is 7 chars wide including both quotes.
		let src = "def s = \"hello\"\n";
		let toks = classify(src);
		assert!(
			toks.contains(&(0, 8, 7, STRING)),
			"quoted string: {:?}",
			toks
		);
	}

	#[test]
	fn interpolated_string_quotes_attach_to_ends() {
		// `"hi $(name)"`: the opening quote joins the leading piece and the
		// closing quote the trailing piece; the `$(` / `)` stay OPERATOR and
		// `name` stays a VARIABLE. The interior pieces must not swallow a quote.
		let src = "def s = \"hi $(name)\"\n";
		let toks = classify(src);
		// Leading piece `"hi ` — opening quote (col 8) + `hi ` = 4 chars.
		assert!(
			toks.contains(&(0, 8, 4, STRING)),
			"leading piece: {:?}",
			toks
		);
		// Trailing piece — just the closing quote, 1 char.
		assert!(
			toks.contains(&(0, 19, 1, STRING)),
			"closing quote: {:?}",
			toks
		);
		assert!(
			toks.contains(&(0, 14, 4, VARIABLE)),
			"name in interp: {:?}",
			toks
		);
	}

	#[test]
	fn comments_are_highlighted() {
		// Full-line comment and a trailing comment after code. The range
		// spans the `#` through end of line (the `#` is included).
		let src = "# a full-line comment\ndef x = 1 # trailing\n";
		let toks = classify(src);
		assert!(
			toks.contains(&(0, 0, 21, COMMENT)),
			"full-line comment: {:?}",
			toks
		);
		assert!(
			toks.contains(&(1, 10, 10, COMMENT)),
			"trailing comment: {:?}",
			toks
		);
		// The code before the trailing comment still classifies normally.
		assert!(toks.contains(&(1, 0, 3, KEYWORD)), "def kw");
		assert!(toks.contains(&(1, 8, 1, NUMBER)), "1 literal");
	}
}
