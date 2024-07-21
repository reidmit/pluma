use compiler::{Token, Tokenizer};
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType};

pub const TOKEN_TYPES: &[SemanticTokenType] = &[
	SemanticTokenType::COMMENT,  // 0
	SemanticTokenType::FUNCTION, // 1
	SemanticTokenType::VARIABLE, // 2
	SemanticTokenType::STRING,   // 3
	SemanticTokenType::NUMBER,   // 4
	SemanticTokenType::REGEXP,   // 5
	SemanticTokenType::KEYWORD,  // 6
	SemanticTokenType::OPERATOR, // 7
	                             // TODO: others here?
];

fn token_legend_index(token: &Token) -> Option<u32> {
	match token {
		Token::Comment(..) => Some(0),
		Token::StringLiteral(..) => Some(3),
		Token::DecimalDigits(..) | Token::OctalDigits(..) | Token::BinaryDigits(..) => Some(4),
		Token::KeywordDef(..) | Token::KeywordIf(..) => Some(6),
		_ => None,
	}
}

pub fn collect_semantic_tokens(source: &Vec<u8>) -> Option<Vec<SemanticToken>> {
	let mut tokenizer = Tokenizer::from_source(source);
	let mut semantic_tokens = Vec::new();
	let mut current_line = 0;
	let mut last_line_end = 0;

	loop {
		match tokenizer.next() {
			None => break,
			Some(token) => {
				if token.is_line_break() {
					last_line_end = token.get_span().1;
					current_line += 1;
					continue;
				}

				if let Some(token_type) = token_legend_index(&token) {
					let (start, end) = token.get_span();

					semantic_tokens.push(SemanticToken {
						delta_line: current_line,
						delta_start: (start - last_line_end) as u32,
						length: (end - start) as u32,
						token_type,
						token_modifiers_bitset: 0,
					});
				}
			}
		}
	}

	Some(semantic_tokens)
}
