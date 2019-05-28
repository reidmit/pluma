#ifndef hum_lexer_h
#define hum_lexer_h

typedef struct {
  const char* start;
  const char* current;
  int line;
} Lexer;

typedef enum {
  TOKEN_EOF,
  TOKEN_ERROR,

  TOKEN_LEFT_PAREN,
  TOKEN_RIGHT_PAREN,
  TOKEN_LEFT_BRACE,
  TOKEN_RIGHT_BRACE,
  TOKEN_LEFT_BRACKET,
  TOKEN_RIGHT_BRACKET,
  TOKEN_COMMA,
  TOKEN_DOT,
  TOKEN_COLON,
  TOKEN_EQUALS,

  TOKEN_ARROW,
  TOKEN_DOUBLE_ARROW,
  TOKEN_DOUBLE_COLON,
  TOKEN_COLON_EQUALS,

  TOKEN_IDENTIFIER,
  TOKEN_COMMENT,
  TOKEN_NUMBER,
  TOKEN_STRING,
} TokenType;

typedef struct {
  TokenType type;
  const char* start;
  int length;
  int line;
} Token;

Lexer* newLexer(const char* source);
void freeLexer(Lexer* lexer);
Token readToken(Lexer* lexer);

#endif