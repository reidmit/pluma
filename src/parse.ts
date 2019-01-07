import { ParseError } from './errors';

class Parser {
  private readonly tokens: Token[];
  private readonly source: string;
  private readonly tokenCount: number;
  private index: number;
  private token: Token;
  private eof: boolean;

  constructor(tokens: Token[], source: string) {
    this.tokens = tokens;
    this.source = source;
    this.tokenCount = this.tokens.length;
    this.index = 0;
    this.token = this.tokens[this.index];
    this.eof = false;
  }

  private advance(amount: number = 1) {
    this.index += amount;
    this.token = this.tokens[this.index];
    if (this.index >= this.tokenCount) this.eof = true;
  }

  private tokenIs(kind: TokenKind, nextKind?: TokenKind) {
    if (this.token.kind !== kind) return false;

    if (nextKind) {
      const nextToken = this.tokens[this.index + 1];
      if (!nextToken) return false;
      if (nextToken.kind !== nextKind) return false;
    }

    return true;
  }

  private parseComments(): void {
    while (!this.eof && this.tokenIs('Comment')) {
      this.advance();
    }
  }

  private parseParenthetical(): Expression {
    if (!this.tokenIs('LeftParen')) return;

    const { lineStart, colStart } = this.token;

    this.advance();

    const expr = this.parseExpression();

    if (!this.tokenIs('RightParen')) {
      throw new ParseError(
        this.token.lineStart,
        this.token.lineEnd,
        `Expected a ) to match ( at line ${lineStart}, column ${colStart}, but found ${
          this.token.kind
        }`
      );
    }

    this.advance();

    return expr;
  }

  private parseAssignmentExpression(): AssignmentExpression {
    if (!this.tokenIs('Identifier', 'Equals')) return;

    const id = this.parseIdentifier();

    if (/^[A-Z]/.test(id.name)) {
      throw new ParseError(
        id.lineStart,
        id.colStart,
        `Cannot assign to type name '${
          id.name
        }'. If you meant to declare a type, use the 'type' keyword. ` +
          `If you meant to assign to a variable, it must start with a lowercase letter.`
      );
    }

    this.advance();

    const expr = this.parseExpression();

    return {
      kind: 'AssignmentExpression',
      type: null,
      leftSide: id,
      rightSide: expr,
      lineStart: id.lineStart,
      lineEnd: expr.lineEnd,
      colStart: id.colStart,
      colEnd: expr.colEnd
    };
  }

  private parseIdentifier(): Identifier {
    if (!this.tokenIs('Identifier')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;

    this.advance();

    return {
      kind: 'Identifier',
      type: null,
      name: value,
      lineStart,
      lineEnd,
      colStart,
      colEnd
    };
  }

  private parseBlock(): Block {
    if (!this.tokenIs('LeftBrace')) return;

    const { lineStart, colStart } = this.token;

    this.advance();

    const parameters: Identifier[] = [];
    const firstExpression = this.parseExpression();

    let body;

    if (
      firstExpression.kind === 'Identifier' &&
      (this.tokenIs('Comma') || this.tokenIs('DoubleArrow'))
    ) {
      parameters.push(firstExpression);

      while (this.tokenIs('Comma')) {
        this.advance();

        const parameter = this.parseIdentifier();

        if (!parameter) {
          throw new ParseError(-1, -1, 'Expected a parameter name after , in block');
        }

        parameters.push(parameter);
      }

      if (!this.tokenIs('DoubleArrow')) {
        throw new ParseError(
          this.token.lineStart,
          this.token.lineEnd,
          `Expected a => after block parameters, but found ${this.token.kind}`
        );
      }

      this.advance();

      body = this.parseExpression();

      if (!body) {
        throw new ParseError(-1, -1, 'Expected an expression after => in block');
      }
    } else {
      body = firstExpression;
    }

    if (!this.tokenIs('RightBrace')) {
      throw new ParseError(
        this.token.lineStart,
        this.token.lineEnd,
        `Expected a } to close block, but found ${this.token.kind}`
      );
    }

    const { lineEnd, colEnd } = this.token;

    this.advance();

    return {
      kind: 'Block',
      type: null,
      body,
      parameters,
      lineStart,
      lineEnd,
      colStart,
      colEnd
    };
  }

  private parseBooleanLiteral(): BooleanLiteral {
    if (!this.tokenIs('Boolean')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;

    this.advance();

    return {
      kind: 'BooleanLiteral',
      type: null,
      value: value === 'true',
      lineStart,
      lineEnd,
      colStart,
      colEnd
    };
  }

  private parseNumericLiteral(): NumericLiteral {
    if (!this.tokenIs('Number')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    const style = value.indexOf('.') > -1 ? 'float' : 'integer';
    const numericValue = Number(value);

    this.advance();

    return {
      kind: 'NumericLiteral',
      type: null,
      style,
      value: numericValue,
      rawValue: value,
      lineStart,
      lineEnd,
      colStart,
      colEnd
    };
  }

  parseStringLiteral(): StringLiteral {
    if (!this.tokenIs('String')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;

    this.advance();

    return {
      kind: 'StringLiteral',
      type: null,
      value,
      lineStart,
      lineEnd,
      colStart,
      colEnd
    };
  }

  parseExpression(): Expression {
    if (this.eof) return;

    this.parseComments();

    if (!this.token) return;

    return (
      this.parseParenthetical() ||
      this.parseAssignmentExpression() ||
      this.parseBlock() ||
      this.parseIdentifier() ||
      this.parseBooleanLiteral() ||
      this.parseNumericLiteral() ||
      this.parseStringLiteral()
    );
  }

  parseModule(): Module {
    const definitions: Definition[] = [];
    const body: Expression[] = [];

    let node;

    while ((node = this.parseExpression())) {
      body.push(node);
    }

    return {
      kind: 'Module',
      type: null,
      definitions,
      body,
      lineStart: 0,
      lineEnd: 0,
      colStart: 0,
      colEnd: 0
    };
  }
}

export function parseExpression(tokens: Token[], source: string): Expression {
  return new Parser(tokens, source).parseExpression();
}

export function parseModule(tokens: Token[], source: string): Module {
  return new Parser(tokens, source).parseModule();
}
