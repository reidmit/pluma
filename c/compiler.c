#include "lexer.h"
#include <stdio.h>

void compile(const char* source) {
  int line = -1;

  Lexer* lexer = newLexer(source);

  for (;;) {
    Token token = readToken(lexer);

    if (token.line != line) {
      printf("%4d ", token.line);
      line = token.line;
    } else {
      printf("   | ");
    }

    printf("%2d '%.*s'\n", token.type, token.length, token.start);

    if (token.type == TOKEN_EOF) {
      break;
    }
  }

  freeLexer(lexer);
}