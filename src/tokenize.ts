import * as t from './tokens';
import { ParseError } from './errors';
export { tokenize };

function tokenize(source: string): t.Token[] {
  const tok = new Tokenizer(source);

  let token;
  while ((token = readToken(tok))) {
    if (Array.isArray(token)) tok.tokens.push(...token);
    else tok.tokens.push(token);
  }

  return tok.tokens;
}

class Tokenizer {
  readonly source: string;
  readonly chars: string[];
  readonly length: number;
  readonly tokens: t.Token[];
  index: number;
  char: string;
  line: number;
  lineStartIndex: number;
  eof: boolean;

  constructor(source: string) {
    this.source = source;
    this.chars = Array.from(this.source);
    this.length = this.chars.length;
    this.tokens = [];
    this.char = this.chars[0];
    this.index = 0;
    this.line = 1;
    this.lineStartIndex = 0;
    this.eof = false;
  }
}

function advance(tok: Tokenizer, amount = 1) {
  tok.index += amount;
  tok.char = tok.chars[tok.index];
  if (tok.index >= tok.length) tok.eof = true;
}

function charIs(tok: Tokenizer, char: string, nextChar?: string, nextNextChar?: string) {
  if (tok.char !== char) return false;
  if (nextChar && tok.chars[tok.index + 1] !== nextChar) return false;
  if (nextNextChar && tok.chars[tok.index + 2] !== nextChar) return false;
  return true;
}

function prevCharIs(tok: Tokenizer, testChar: string) {
  return tok.chars[tok.index - 1] === testChar;
}

function readComment(tok: Tokenizer): t.Token | void {
  if (!charIs(tok, '#')) return;

  const colStart = tok.index - tok.lineStartIndex;
  let value = '';
  let skip = true;

  while (!charIs(tok, '\n') && !tok.eof) {
    if (!skip) value += tok.char;
    advance(tok);
    skip = false;
  }

  const colEnd = tok.index - tok.lineStartIndex;

  return {
    kind: 'Comment',
    value,
    lineStart: tok.line,
    lineEnd: tok.line,
    colStart,
    colEnd
  };
}

function readIdentifier(tok: Tokenizer): t.Token | void {
  if (!isIdentifierStartChar(tok.char)) return;

  let value = '';
  const colStart = tok.index - tok.lineStartIndex;

  while (isIdentifierChar(tok.char) && !tok.eof) {
    value += tok.char;
    advance(tok);
  }

  const colEnd = tok.index - tok.lineStartIndex;

  if (value === 'true' || value === 'false') {
    return {
      kind: 'Boolean',
      value,
      lineStart: tok.line,
      lineEnd: tok.line,
      colStart,
      colEnd
    };
  }

  return {
    kind: 'Identifier',
    value,
    lineStart: tok.line,
    lineEnd: tok.line,
    colStart,
    colEnd
  };
}

function readChar(tok: Tokenizer): t.Token | void {
  if (!charIs(tok, "'")) return;

  const colStart = tok.index - tok.lineStartIndex;

  advance(tok);

  const value = tok.char;

  advance(tok);

  if (!charIs(tok, "'")) {
    throw new ParseError(
      tok.line,
      tok.index - tok.lineStartIndex,
      "Expected closing ' after character"
    );
  }

  const colEnd = tok.index - tok.lineStartIndex;

  advance(tok);

  return {
    kind: 'Char',
    value,
    lineStart: tok.line,
    lineEnd: tok.line,
    colStart,
    colEnd
  };
}

function readNumber(tok: Tokenizer): t.Token | void {
  if (!isDecimalDigit(tok.char)) return;

  const colStart = tok.index - tok.lineStartIndex;

  let value = '';

  if (charIs(tok, '0', 'x')) {
    value += '0x';
    advance(tok, 2);

    while (isHexDigit(tok.char) && !tok.eof) {
      value += tok.char;
      advance(tok);
    }

    const colEnd = tok.index - tok.lineStartIndex;

    return {
      kind: 'Number',
      value,
      lineStart: tok.line,
      lineEnd: tok.line,
      colStart,
      colEnd
    };
  }

  if (charIs(tok, '0', 'o')) {
    value += '0o';
    advance(tok, 2);

    while (isOctalDigit(tok.char) && !tok.eof) {
      value += tok.char;
      advance(tok);
    }

    const colEnd = tok.index - tok.lineStartIndex;

    return {
      kind: 'Number',
      value,
      lineStart: tok.line,
      lineEnd: tok.line,
      colStart,
      colEnd
    };
  }

  if (charIs(tok, '0', 'b')) {
    value += '0b';
    advance(tok, 2);

    while (isBinaryDigit(tok.char) && !tok.eof) {
      value += tok.char;
      advance(tok);
    }

    const colEnd = tok.index - tok.lineStartIndex;

    return {
      kind: 'Number',
      value,
      lineStart: tok.line,
      lineEnd: tok.line,
      colStart,
      colEnd
    };
  }

  while (isDecimalDigit(tok.char) && !tok.eof) {
    value += tok.char;
    advance(tok);
  }

  if (charIs(tok, '.')) {
    value += tok.char;
    advance(tok);

    while (isDecimalDigit(tok.char) && !tok.eof) {
      value += tok.char;
      advance(tok);
    }
  }

  if (charIs(tok, 'e')) {
    value += tok.char;
    advance(tok);

    while (isDecimalDigit(tok.char) && !tok.eof) {
      value += tok.char;
      advance(tok);
    }
  }

  const colEnd = tok.index - tok.lineStartIndex;

  return {
    kind: 'Number',
    value,
    lineStart: tok.line,
    lineEnd: tok.line,
    colStart,
    colEnd
  };
}

function readString(tok: Tokenizer): t.Token[] | void {
  if (!charIs(tok, '"')) return;

  const tripleQuoted = charIs(tok, '"', '"', '"');
  const colStart = tok.index - tok.lineStartIndex;
  const quoteSize = tripleQuoted ? 3 : 1;
  const stringTokens: t.Token[] = [];

  let lineStart = tok.line;
  let value = '';

  advance(tok, quoteSize);

  while (!tok.eof) {
    if (charIs(tok, '\n')) {
      tok.line++;
      tok.lineStartIndex = tok.index + 1;
    }

    if (!tripleQuoted) {
      if (charIs(tok, '"') && !prevCharIs(tok, '\\')) break;
    } else {
      if (charIs(tok, '"', '"', '"')) break;
    }

    if (charIs(tok, '$', '(') && !prevCharIs(tok, '\\')) {
      const col = tok.index - tok.lineStartIndex;

      advance(tok, 2);

      stringTokens.push({
        kind: 'String',
        value,
        lineStart,
        lineEnd: tok.line,
        colStart,
        colEnd: col
      });

      stringTokens.push({
        kind: 'InterpolationStart',
        value: '$(',
        lineStart: tok.line,
        lineEnd: tok.line,
        colStart: col,
        colEnd: col + 2
      });

      value = '';

      const parenStack = [];

      let innerToken;

      while ((innerToken = readToken(tok))) {
        if (Array.isArray(innerToken)) {
          stringTokens.push(...innerToken);
          continue;
        }

        if (innerToken.kind === '(') {
          parenStack.push(true);
        } else if (innerToken.kind === ')') {
          if (!parenStack.length) {
            stringTokens.push({
              kind: 'InterpolationEnd',
              value: ')',
              lineStart: tok.line,
              lineEnd: tok.line,
              colStart: innerToken.colStart,
              colEnd: innerToken.colEnd
            });

            lineStart = tok.line;

            break;
          }

          parenStack.pop();
        }

        stringTokens.push(innerToken);
      }
    } else {
      value += tok.char;
      advance(tok);
    }
  }

  if (!tripleQuoted && !charIs(tok, '"')) {
    const col = tok.index - tok.lineStartIndex;

    throw new ParseError(
      tok.line,
      col,
      `Missing a closing " for string starting on line ${lineStart}`
    );
  }

  if (tripleQuoted && !charIs(tok, '"', '"', '"')) {
    const col = tok.index - tok.lineStartIndex;

    throw new ParseError(
      tok.line,
      col,
      `Missing a closing """ for string starting on line ${lineStart}`
    );
  }

  advance(tok, quoteSize);

  const colEnd = tok.index - tok.lineStartIndex;

  stringTokens.push({
    kind: 'String',
    value,
    lineStart,
    lineEnd: tok.line,
    colStart,
    colEnd
  });

  return stringTokens;
}

function readSymbol(
  tok: Tokenizer,
  kind: t.TokenKind,
  firstChar: string,
  secondChar?: string
): t.Token | void {
  if (!charIs(tok, firstChar, secondChar)) return;

  const size = secondChar ? 2 : 1;
  const colStart = tok.index - tok.lineStartIndex;

  advance(tok, size);

  return {
    kind,
    value: kind,
    lineStart: tok.line,
    lineEnd: tok.line,
    colStart,
    colEnd: colStart + size
  };
}

function readOperator(tok: Tokenizer): t.Token | void {
  if (!isOperatorChar(tok.char)) return;

  const colStart = tok.index - tok.lineStartIndex;

  let value = '';

  while (isOperatorChar(tok.char)) {
    value += tok.char;
    advance(tok);
  }

  const colEnd = tok.index - tok.lineStartIndex;

  return {
    kind: 'Operator',
    value,
    lineStart: tok.line,
    lineEnd: tok.line,
    colStart,
    colEnd
  };
}

function readWhitespace(tok: Tokenizer) {
  while (isWhitespace(tok.char) && !tok.eof) {
    if (charIs(tok, '\n')) {
      tok.line++;
      tok.lineStartIndex = tok.index + 1;
    }

    advance(tok);
  }
}

function readToken(tok: Tokenizer): t.Token | t.Token[] | void {
  if (tok.eof) return;

  readWhitespace(tok);

  return (
    readIdentifier(tok) ||
    readNumber(tok) ||
    readChar(tok) ||
    readString(tok) ||
    readSymbol(tok, '->', '-', '>') ||
    readSymbol(tok, ':=', ':', '=') ||
    readSymbol(tok, '=>', '=', '>') ||
    readSymbol(tok, '{', '{') ||
    readSymbol(tok, '}', '}') ||
    readSymbol(tok, '(', '(') ||
    readSymbol(tok, ')', ')') ||
    readSymbol(tok, '[', '[') ||
    readSymbol(tok, ']', ']') ||
    readSymbol(tok, '.', '.') ||
    readSymbol(tok, ',', ',') ||
    readSymbol(tok, ':', ':') ||
    readSymbol(tok, '=', '=') ||
    readOperator(tok) ||
    readComment(tok)
  );
}

function isWhitespace(char: string) {
  switch (char) {
    case ' ':
    case '\n':
    case '\t':
    case '\r':
      return true;
    default:
      return false;
  }
}

function isIdentifierStartChar(char: string) {
  if (!char) return false;

  const codePoint = char.codePointAt(0) || -1;

  return (
    (codePoint >= 97 && codePoint <= 122) ||
    (codePoint >= 65 && codePoint <= 90) ||
    codePoint === 95
  );
}

function isIdentifierChar(char: string) {
  return isIdentifierStartChar(char) || isDecimalDigit(char);
}

function isDecimalDigit(char: string) {
  switch (char) {
    case '0':
    case '1':
    case '2':
    case '3':
    case '4':
    case '5':
    case '6':
    case '7':
    case '8':
    case '9':
      return true;
    default:
      return false;
  }
}

function isHexDigit(char: string) {
  return (
    isDecimalDigit(char) || (char >= 'a' && char <= 'f') || (char >= 'A' && char <= 'F')
  );
}

function isOctalDigit(char: string) {
  switch (char) {
    case '0':
    case '1':
    case '2':
    case '3':
    case '4':
    case '5':
    case '6':
    case '7':
      return true;
    default:
      return false;
  }
}

function isBinaryDigit(char: string) {
  switch (char) {
    case '0':
    case '1':
      return true;
    default:
      return false;
  }
}

function isOperatorChar(char: string) {
  switch (char) {
    case '@':
    case '<':
    case '>':
    case '=':
    case '|':
    case '!':
    case '+':
    case '*':
    case '-':
      return true;
    default:
      return false;
  }
}
