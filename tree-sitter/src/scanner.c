#include "tree_sitter/parser.h"

// External scanner for Pluma's statement terminator.
//
// Pluma separates statements with newlines (there is no `;`). We emit a single
// `_newline` token when we cross one or more newlines AND the parser is in a
// state that expects a terminator (`valid_symbols[NEWLINE]`). Where a newline
// is not a valid terminator — inside `(...)`/`[...]`, or mid-expression right
// after a binary operator — `valid_symbols[NEWLINE]` is false, so we consume
// the newline as ordinary whitespace and line continuation happens for free.
//
// The scanner is stateless: all context lives in the parse state, surfaced via
// `valid_symbols`.

enum TokenType {
  NEWLINE,
};

void *tree_sitter_pluma_external_scanner_create() { return NULL; }
void tree_sitter_pluma_external_scanner_destroy(void *payload) {}
unsigned tree_sitter_pluma_external_scanner_serialize(void *payload, char *buffer) { return 0; }
void tree_sitter_pluma_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {}

bool tree_sitter_pluma_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  bool saw_newline = false;

  // Skip whitespace; remember whether any of it was a line break.
  for (;;) {
    int32_t c = lexer->lookahead;
    if (c == ' ' || c == '\t' || c == '\r') {
      lexer->advance(lexer, true);
    } else if (c == '\n') {
      saw_newline = true;
      lexer->advance(lexer, true);
    } else {
      break;
    }
  }

  if (saw_newline && valid_symbols[NEWLINE]) {
    lexer->result_symbol = NEWLINE;
    return true;
  }

  // Either no newline, or a newline that isn't a valid terminator here: the
  // whitespace has been consumed as a skip, so hand back to the normal lexer.
  return false;
}
