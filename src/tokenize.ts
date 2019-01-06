function isWhitespace(char: string) {
  return /^[ \n\r\t]/.test(char);
}

function isIdentifierStartChar(char: string) {
  return (
    (char >= 'a' && char <= 'z') || (char >= 'A' && char <= 'Z') || char === '_'
  );
}

function isIdentifierChar(char: string) {
  return isIdentifierStartChar(char) || isDecimalDigit(char);
}

function isDecimalDigit(char: string) {
  return char >= '0' && char <= '9';
}

function isHexDigit(char: string) {
  return (
    isDecimalDigit(char) ||
    (char >= 'a' && char <= 'f') ||
    (char >= 'A' && char <= 'F')
  );
}

function isOctalDigit(char: string) {
  return char >= '0' && char <= '7';
}

function isBinaryDigit(char: string) {
  return char === '0' || char === '1';
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
  }

  return false;
}

class Tokenizer {
  source: string;
  chars: string[];
  index: number;
  char: string;
  tokens: Token[];
  line: number;
  lineStartIndex: number;
  eof: boolean;

  constructor(source: string) {
    this.source = source;
    this.chars = Array.from(this.source);
    this.char = this.chars[0];
    this.index = 0;
    this.tokens = [];
    this.line = 1;
    this.lineStartIndex = 0;
    this.eof = false;
  }

  advance(amount = 1) {
    this.index += amount;
    this.char = this.chars[this.index];
    if (this.index >= this.chars.length) this.eof = true;
  }

  charIs(testChar: string) {
    return this.char === testChar;
  }

  nextCharIs(testChar: string) {
    return this.chars[this.index + 1] === testChar;
  }

  nextNextCharIs(testChar: string) {
    return this.chars[this.index + 2] === testChar;
  }

  prevCharIs(testChar: string) {
    return this.chars[this.index - 1] === testChar;
  }

  readComment(): Token {
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

  readIdentifier(): Token {
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

  readNumber(): Token {
    if (!isDecimalDigit(this.char)) return;

    const colStart = this.index - this.lineStartIndex;

    let value = '';

    if (this.charIs('0') && this.nextCharIs('x')) {
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

    if (this.charIs('0') && this.nextCharIs('o')) {
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

    if (this.charIs('0') && this.nextCharIs('b')) {
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

  readString(): Token[] {
    if (!this.charIs('"')) return;

    const colStart = this.index - this.lineStartIndex;

    let lineStart = this.line;
    const tripleQuoted = this.nextCharIs('"') && this.nextNextCharIs('"');

    const stringTokens: Token[] = [];
    let value = '';

    this.advance(tripleQuoted ? 3 : 1);

    while (!this.eof) {
      if (this.charIs('\n')) {
        this.line++;
        this.lineStartIndex = this.index + 1;
      }

      if (!tripleQuoted) {
        if (this.charIs('"') && !this.prevCharIs('\\')) break;
      } else {
        if (
          this.charIs('"') &&
          this.nextCharIs('"') &&
          this.nextNextCharIs('"')
        )
          break;
      }

      if (this.charIs('$') && this.nextCharIs('(') && !this.prevCharIs('\\')) {
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

        while ((innerToken = this.nextToken())) {
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

        if (this.eof) {
          throw new SyntaxError('unexpected EOF');
        }
      } else {
        value += this.char;
        this.advance();
      }
    }

    if (!tripleQuoted && !this.charIs('"')) {
      throw new SyntaxError('unclosed string, expected "');
    }

    if (
      tripleQuoted &&
      (!this.charIs('"') || !this.nextCharIs('"') || !this.nextNextCharIs('"'))
    ) {
      throw new SyntaxError('unclosed triple-quoted string, expected """');
    }

    const quoteSize = tripleQuoted ? 3 : 1;
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

  readSymbol(kind: TokenKind, firstChar: string, secondChar?: string): Token {
    if (!this.charIs(firstChar)) return;
    if (secondChar && !this.nextCharIs(secondChar)) return;

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

  readOperator(): Token {
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

  readWhitespace() {
    while (isWhitespace(this.char) && !this.eof) {
      if (this.charIs('\n')) {
        this.line++;
        this.lineStartIndex = this.index + 1;
      }

      this.advance();
    }
  }

  nextToken(): Token | Token[] {
    if (this.eof) return;

    this.readWhitespace();

    return (
      this.readIdentifier() ||
      this.readNumber() ||
      this.readString() ||
      this.readSymbol('Arrow', '-', '>') ||
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

  tokenize(): Token[] {
    let token;

    while ((token = this.nextToken())) {
      if (Array.isArray(token)) this.tokens.push(...token);
      else this.tokens.push(token);
    }

    return this.tokens;
  }
}

export function tokenize(source: string): Token[] {
  return new Tokenizer(source).tokenize();
}
