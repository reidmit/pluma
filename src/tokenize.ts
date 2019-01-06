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
  return (
    char === '@' ||
    char === '<' ||
    char === '>' ||
    char === '=' ||
    char === '|' ||
    char === '!' ||
    char === '+' ||
    char === '*' ||
    char === '-'
  );
}

class Tokenizer {
  input: string;
  chars: string[];
  index: number;
  char: string;
  tokens: Token[];
  line: number;
  lineStartIndex: number;
  eof: boolean;

  constructor(input: string) {
    this.input = input;
    this.chars = Array.from(this.input);
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

  comment(): Token {
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
      kind: 'comment',
      value,
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      }
    };
  }

  identifier(): Token {
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
        kind: 'boolean',
        value,
        location: {
          lineStart: this.line,
          lineEnd: this.line,
          colStart,
          colEnd
        }
      };
    }

    return {
      kind: 'identifier',
      value,
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      }
    };
  }

  number(): Token {
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
        kind: 'number',
        value,
        location: {
          lineStart: this.line,
          lineEnd: this.line,
          colStart,
          colEnd
        }
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
        kind: 'number',
        value,
        location: {
          lineStart: this.line,
          lineEnd: this.line,
          colStart,
          colEnd
        }
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
        kind: 'number',
        value,
        location: {
          lineStart: this.line,
          lineEnd: this.line,
          colStart,
          colEnd
        }
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
      kind: 'number',
      value,
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      }
    };
  }

  string(): Token[] {
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
          kind: 'string',
          value,
          location: {
            lineStart,
            lineEnd: this.line,
            colStart,
            colEnd: col
          }
        });

        stringTokens.push({
          kind: 'interpolation-start',
          location: {
            lineStart: this.line,
            lineEnd: this.line,
            colStart: col,
            colEnd: col + 2
          }
        });

        value = '';

        const parenStack = [];

        let innerToken;

        while ((innerToken = this.nextToken())) {
          if (Array.isArray(innerToken)) {
            stringTokens.push(...innerToken);
            continue;
          }

          if (innerToken.kind === 'l-paren') {
            parenStack.push(true);
          } else if (innerToken.kind === 'r-paren') {
            if (!parenStack.length) {
              stringTokens.push({
                kind: 'interpolation-end',
                location: {
                  lineStart: this.line,
                  lineEnd: this.line,
                  colStart: innerToken.location.colStart,
                  colEnd: innerToken.location.colEnd
                }
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
      kind: 'string',
      value,
      location: {
        lineStart,
        lineEnd: this.line,
        colStart,
        colEnd
      }
    });

    return stringTokens;
  }

  lBrace(): Token {
    if (!this.charIs('{')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'l-brace',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  rBrace(): Token {
    if (!this.charIs('}')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'r-brace',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  lBracket(): Token {
    if (!this.charIs('[')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'l-bracket',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  rBracket(): Token {
    if (!this.charIs(']')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'r-bracket',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  lParen(): Token {
    if (!this.charIs('(')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'l-paren',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  rParen(): Token {
    if (!this.charIs(')')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'r-paren',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  dot(): Token {
    if (!this.charIs('.')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'dot',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  comma(): Token {
    if (!this.charIs(',')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'comma',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  colon(): Token {
    if (!this.charIs(':')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'colon',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  equals(): Token {
    if (!this.charIs('=')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance();

    return {
      kind: 'equals',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 1
      }
    };
  }

  doubleArrow(): Token {
    if (!this.charIs('=') || !this.nextCharIs('>')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance(2);

    return {
      kind: 'double-arrow',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 2
      }
    };
  }

  arrow(): Token {
    if (!this.charIs('-') || !this.nextCharIs('>')) return;

    const colStart = this.index - this.lineStartIndex;

    this.advance(2);

    return {
      kind: 'arrow',
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd: colStart + 2
      }
    };
  }

  operator(): Token {
    if (!isOperatorChar(this.char)) return;

    const colStart = this.index - this.lineStartIndex;

    let value = '';

    while (isOperatorChar(this.char)) {
      value += this.char;
      this.advance();
    }

    const colEnd = this.index - this.lineStartIndex;

    return {
      kind: 'operator',
      value,
      location: {
        lineStart: this.line,
        lineEnd: this.line,
        colStart,
        colEnd
      }
    };
  }

  whitespace() {
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

    this.whitespace();

    return (
      this.identifier() ||
      this.number() ||
      this.string() ||
      this.lBrace() ||
      this.rBrace() ||
      this.lParen() ||
      this.rParen() ||
      this.lBracket() ||
      this.rBracket() ||
      this.dot() ||
      this.comma() ||
      this.doubleArrow() ||
      this.arrow() ||
      this.colon() ||
      this.equals() ||
      this.operator() ||
      this.comment()
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

export function tokenize(input: string): Token[] {
  return new Tokenizer(input).tokenize();
}
