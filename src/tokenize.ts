import { ParseError } from './errors';
import * as t from './types';

class Tokenizer {
  private readonly source: string;
  private readonly chars: string[];
  private readonly length: number;
  private readonly tokens: t.Token[];
  private index: number;
  private char: string;
  private line: number;
  private lineStartIndex: number;
  private eof: boolean;

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

  private advance(amount = 1) {
    this.index += amount;
    this.char = this.chars[this.index];
    if (this.index >= this.length) this.eof = true;
  }

  private charIs(char: string, nextChar?: string, nextNextChar?: string) {
    if (this.char !== char) return false;
    if (nextChar && this.chars[this.index + 1] !== nextChar) return false;
    if (nextNextChar && this.chars[this.index + 2] !== nextChar) return false;
    return true;
  }

  private prevCharIs(testChar: string) {
    return this.chars[this.index - 1] === testChar;
  }

  private readComment(): t.Token {
    if (!this.charIs('#')) return;

    const colStart = this.index - this.lineStartIndex;
    let value = '';
    let skip = true;

    while (!this.charIs('\n') && !this.eof) {
      if (!skip) value += this.char;
      this.advance();
      skip = false;
    }

    const colEnd = this.index - this.lineStartIndex;

    return {
      kind: 'Comment',
      value,
      lineStart: this.line,
      lineEnd: this.line,
      colStart,
      colEnd
    };
  }

  private readIdentifier(): t.Token {
    if (!isIdentifierStartChar(this.char)) return;

    let value = '';
    const colStart = this.index - this.lineStartIndex;

    while (isIdentifierChar(this.char) && !this.eof) {
      value += this.char;
      this.advance();
    }

    const colEnd = this.index - this.lineStartIndex;

    if (value === 'true' || value === 'false') {
      return {
        kind: 'Boolean',
        value,
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      };
    }

    return {
      kind: 'Identifier',
      value,
      lineStart: this.line,
      lineEnd: this.line,
      colStart,
      colEnd
    };
  }

  private readChar(): t.Token {
    if (!this.charIs("'")) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    const value = this.char;

    this.advance();

    if (!this.charIs("'")) {
      throw new ParseError(
        this.line,
        this.index - this.lineStartIndex,
        "Expected closing ' after character"
      );
    }

    const colEnd = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'Char',
      value,
      lineStart: this.line,
      lineEnd: this.line,
      colStart,
      colEnd
    };
  }

  private readNumber(): t.Token {
    if (!isDecimalDigit(this.char)) return;

    const colStart = this.index - this.lineStartIndex;

    let value = '';

    if (this.charIs('0', 'x')) {
      value += '0x';
      this.advance(2);

      while (isHexDigit(this.char) && !this.eof) {
        value += this.char;
        this.advance();
      }

      const colEnd = this.index - this.lineStartIndex;

      return {
        kind: 'Number',
        value,
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      };
    }

    if (this.charIs('0', 'o')) {
      value += '0o';
      this.advance(2);

      while (isOctalDigit(this.char) && !this.eof) {
        value += this.char;
        this.advance();
      }

      const colEnd = this.index - this.lineStartIndex;

      return {
        kind: 'Number',
        value,
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      };
    }

    if (this.charIs('0', 'b')) {
      value += '0b';
      this.advance(2);

      while (isBinaryDigit(this.char) && !this.eof) {
        value += this.char;
        this.advance();
      }

      const colEnd = this.index - this.lineStartIndex;

      return {
        kind: 'Number',
        value,
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      };
    }

    while (isDecimalDigit(this.char) && !this.eof) {
      value += this.char;
      this.advance();
    }

    if (this.charIs('.')) {
      value += this.char;
      this.advance();

      while (isDecimalDigit(this.char) && !this.eof) {
        value += this.char;
        this.advance();
      }
    }

    if (this.charIs('e')) {
      value += this.char;
      this.advance();

      while (isDecimalDigit(this.char) && !this.eof) {
        value += this.char;
        this.advance();
      }
    }

    const colEnd = this.index - this.lineStartIndex;

    return {
      kind: 'Number',
      value,
      lineStart: this.line,
      lineEnd: this.line,
      colStart,
      colEnd
    };
  }

  private readString(): t.Token[] {
    if (!this.charIs('"')) return;

    const tripleQuoted = this.charIs('"', '"', '"');
    const colStart = this.index - this.lineStartIndex;
    const quoteSize = tripleQuoted ? 3 : 1;
    const stringTokens: t.Token[] = [];

    let lineStart = this.line;
    let value = '';

    this.advance(quoteSize);

    while (!this.eof) {
      if (this.charIs('\n')) {
        this.line++;
        this.lineStartIndex = this.index + 1;
      }

      if (!tripleQuoted) {
        if (this.charIs('"') && !this.prevCharIs('\\')) break;
      } else {
        if (this.charIs('"', '"', '"')) break;
      }

      if (this.charIs('$', '(') && !this.prevCharIs('\\')) {
        const col = this.index - this.lineStartIndex;

        this.advance(2);

        stringTokens.push({
          kind: 'String',
          value,
          lineStart,
          lineEnd: this.line,
          colStart,
          colEnd: col
        });

        stringTokens.push({
          kind: 'InterpolationStart',
          lineStart: this.line,
          lineEnd: this.line,
          colStart: col,
          colEnd: col + 2
        });

        value = '';

        const parenStack = [];

        let innerToken;

        while ((innerToken = this.readToken())) {
          if (Array.isArray(innerToken)) {
            stringTokens.push(...innerToken);
            continue;
          }

          if (innerToken.kind === 'LeftParen') {
            parenStack.push(true);
          } else if (innerToken.kind === 'RightParen') {
            if (!parenStack.length) {
              stringTokens.push({
                kind: 'InterpolationEnd',
                lineStart: this.line,
                lineEnd: this.line,
                colStart: innerToken.colStart,
                colEnd: innerToken.colEnd
              });

              lineStart = this.line;

              break;
            }

            parenStack.pop();
          }

          stringTokens.push(innerToken);
        }
      } else {
        value += this.char;
        this.advance();
      }
    }

    if (!tripleQuoted && !this.charIs('"')) {
      const col = this.index - this.lineStartIndex;

      throw new ParseError(
        this.line,
        col,
        `Missing a closing " for string starting on line ${lineStart}`
      );
    }

    if (tripleQuoted && !this.charIs('"', '"', '"')) {
      const col = this.index - this.lineStartIndex;

      throw new ParseError(
        this.line,
        col,
        `Missing a closing """ for string starting on line ${lineStart}`
      );
    }

    this.advance(quoteSize);

    const colEnd = this.index - this.lineStartIndex;

    stringTokens.push({
      kind: 'String',
      value,
      lineStart,
      lineEnd: this.line,
      colStart,
      colEnd
    });

    return stringTokens;
  }

  private readSymbol(kind: t.TokenKind, firstChar: string, secondChar?: string): t.Token {
    if (!this.charIs(firstChar, secondChar)) return;

    const size = secondChar ? 2 : 1;
    const colStart = this.index - this.lineStartIndex;

    this.advance(size);

    return {
      kind,
      lineStart: this.line,
      lineEnd: this.line,
      colStart,
      colEnd: colStart + size
    };
  }

  private readOperator(): t.Token {
    if (!isOperatorChar(this.char)) return;

    const colStart = this.index - this.lineStartIndex;

    let value = '';

    while (isOperatorChar(this.char)) {
      value += this.char;
      this.advance();
    }

    const colEnd = this.index - this.lineStartIndex;

    return {
      kind: 'Operator',
      value,
      lineStart: this.line,
      lineEnd: this.line,
      colStart,
      colEnd
    };
  }

  private readWhitespace() {
    while (isWhitespace(this.char) && !this.eof) {
      if (this.charIs('\n')) {
        this.line++;
        this.lineStartIndex = this.index + 1;
      }

      this.advance();
    }
  }

  private readToken(): t.Token | t.Token[] {
    if (this.eof) return;

    this.readWhitespace();

    return (
      this.readIdentifier() ||
      this.readNumber() ||
      this.readChar() ||
      this.readString() ||
      this.readSymbol('Arrow', '-', '>') ||
      this.readSymbol('ColonEquals', ':', '=') ||
      this.readSymbol('DoubleArrow', '=', '>') ||
      this.readSymbol('LeftBrace', '{') ||
      this.readSymbol('RightBrace', '}') ||
      this.readSymbol('LeftParen', '(') ||
      this.readSymbol('RightParen', ')') ||
      this.readSymbol('LeftBracket', '[') ||
      this.readSymbol('RightBracket', ']') ||
      this.readSymbol('Dot', '.') ||
      this.readSymbol('Comma', ',') ||
      this.readSymbol('Colon', ':') ||
      this.readSymbol('Equals', '=') ||
      this.readOperator() ||
      this.readComment()
    );
  }

  tokenize(): t.Token[] {
    let token;

    while ((token = this.readToken())) {
      if (Array.isArray(token)) this.tokens.push(...token);
      else this.tokens.push(token);
    }

    return this.tokens;
  }
}

export function tokenize(source: string): t.Token[] {
  return new Tokenizer(source).tokenize();
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

  const codePoint = char.codePointAt(0);

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
