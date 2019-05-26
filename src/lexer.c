#include "lexer.h"
#include "common.h"
#include "memory.h"
#include <stdio.h>
#include <string.h>

static bool isAtEnd(Lexer* lexer) {
  return *lexer->current == '\0';
}

static Token makeToken(Lexer* lexer, TokenType type) {
  Token token;
  token.type = type;
  token.start = lexer->start;
  token.length = (int)(lexer->current - lexer->start);
  token.line = lexer->line;

  return token;
}

static Token errorToken(Lexer* lexer, const char* message) {
  Token token;
  token.type = TOKEN_ERROR;
  token.start = message;
  token.length = (int)strlen(message);
  token.line = lexer->line;

  return token;
}

static char advance(Lexer* lexer) {
  lexer->current++;
  return lexer->current[-1];
}

static bool match(Lexer* lexer, char expected) {
  if (isAtEnd(lexer)) {
    return false;
  }

  if (*lexer->current != expected) {
    return false;
  }

  lexer->current++;
  return true;
}

static bool isDigit(char ch) {
  return ch >= '0' && ch <= '9';
}

static bool isIdentifierStart(char ch) {
  return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || ch == '_';
}

static bool isIdentifierChar(char ch) {
  return isIdentifierStart(ch) || isDigit(ch);
}

static char peek(Lexer* lexer) {
  return *lexer->current;
}

static char peekNext(Lexer* lexer) {
  if (isAtEnd(lexer)) {
    return '\0';
  }

  return lexer->current[1];
}

static void skipWhitespace(Lexer* lexer) {
  for (;;) {
    char ch = peek(lexer);

    switch (ch) {
      case ' ':
      case '\r':
      case '\t':
        advance(lexer);
        break;

      case '\n':
        lexer->line++;
        advance(lexer);
        break;

      default:
        return;
    }
  }
}

static Token makeStringToken(Lexer* lexer) {
  while (peek(lexer) != '"' && !isAtEnd(lexer)) {
    if (peek(lexer) == '\n') {
      lexer->line++;
    }

    advance(lexer);
  }

  if (isAtEnd(lexer)) {
    return errorToken(lexer, "Unterminated string.");
  }

  advance(lexer);
  return makeToken(lexer, TOKEN_STRING);
}

static Token makeCommentToken(Lexer* lexer) {
  while (peek(lexer) != '\n' && !isAtEnd(lexer)) {
    advance(lexer);
  }

  return makeToken(lexer, TOKEN_COMMENT);
}

static Token makeNumberToken(Lexer* lexer) {
  while (isDigit(peek(lexer))) {
    advance(lexer);
  }

  if (peek(lexer) == '.' && isDigit(peekNext(lexer))) {
    advance(lexer);

    while (isDigit(peek(lexer))) {
      advance(lexer);
    }
  }

  return makeToken(lexer, TOKEN_NUMBER);
}

static Token makeIdentifierToken(Lexer* lexer) {
  while (isIdentifierChar(peek(lexer))) {
    advance(lexer);
  }

  return makeToken(lexer, TOKEN_IDENTIFIER);
}

Lexer* newLexer(const char* source) {
  Lexer* lexer = (Lexer*)reallocate(NULL, 0, sizeof(*lexer));
  memset(lexer, 0, sizeof(Lexer));

  lexer->start = source;
  lexer->current = source;
  lexer->line = 1;

  return lexer;
}

void freeLexer(Lexer* lexer) {
  reallocate(lexer, sizeof(Lexer), 0);
}

Token readToken(Lexer* lexer) {
  skipWhitespace(lexer);

  lexer->start = lexer->current;

  if (isAtEnd(lexer)) {
    return makeToken(lexer, TOKEN_EOF);
  }

  char ch = advance(lexer);

  if (isIdentifierStart(ch)) {
    return makeIdentifierToken(lexer);
  }

  if (isDigit(ch)) {
    return makeNumberToken(lexer);
  }

  switch (ch) {
    case '(':
      return makeToken(lexer, TOKEN_LEFT_PAREN);
    case ')':
      return makeToken(lexer, TOKEN_RIGHT_PAREN);
    case '{':
      return makeToken(lexer, TOKEN_LEFT_BRACE);
    case '}':
      return makeToken(lexer, TOKEN_RIGHT_BRACE);
    case '[':
      return makeToken(lexer, TOKEN_LEFT_BRACKET);
    case ']':
      return makeToken(lexer, TOKEN_LEFT_BRACKET);
    case ',':
      return makeToken(lexer, TOKEN_COMMA);
    case '.':
      return makeToken(lexer, TOKEN_DOT);
    case ':':
      return match(lexer, '=') ? makeToken(lexer, TOKEN_COLON_EQUALS)
                               : match(lexer, ':') ? makeToken(lexer, TOKEN_DOUBLE_COLON)
                                                   : makeToken(lexer, TOKEN_COLON);
    case '=':
      return match(lexer, '>') ? makeToken(lexer, TOKEN_DOUBLE_ARROW)
                               : makeToken(lexer, TOKEN_EQUALS);
    case '-':
      if (match(lexer, '>')) {
        return makeToken(lexer, TOKEN_ARROW);
      }
    case '"':
      return makeStringToken(lexer);
    case '#':
      return makeCommentToken(lexer);
  }

  return errorToken(lexer, "Unexpected character.");
}