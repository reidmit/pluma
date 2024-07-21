use tower_lsp::lsp_types::SemanticTokenType;

pub const TOKEN_TYPES: &[SemanticTokenType] = &[
	SemanticTokenType::FUNCTION,
	SemanticTokenType::VARIABLE,
	SemanticTokenType::STRING,
	SemanticTokenType::COMMENT,
	SemanticTokenType::NUMBER,
	SemanticTokenType::REGEXP,
	SemanticTokenType::KEYWORD,
	SemanticTokenType::OPERATOR,
	SemanticTokenType::PARAMETER,
	// TODO: others here?
];
