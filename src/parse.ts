import { ParseError } from './errors';
import * as t from './types';
import * as nodes from './nodes';

class Parser {
  private readonly tokens: t.Token[];
  private readonly source: string;
  private readonly tokenCount: number;
  private index: number;
  private token: t.Token;
  private eof: boolean;

  constructor(tokens: t.Token[], source: string) {
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

  private tokenIs(kind: t.TokenKind, nextKind?: t.TokenKind) {
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

  private parseParenthetical(): t.Expression {
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

  private parseAssignmentExpression(): t.AssignmentExpression {
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

    return new nodes.AssignmentExpression(
      id,
      expr,
      id.lineStart,
      id.colStart,
      expr.lineEnd,
      expr.colEnd
    );
  }

  private parseIdentifier(): t.Identifier {
    if (!this.tokenIs('Identifier')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;

    this.advance();

    return new nodes.Identifier(value, lineStart, colStart, lineEnd, colEnd);
  }

  private parseBlock(): t.Block {
    if (!this.tokenIs('LeftBrace')) return;

    const { lineStart, colStart } = this.token;

    this.advance();

    const parameters: t.Identifier[] = [];
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

    return new nodes.Block(parameters, body, lineStart, colStart, lineEnd, colEnd);
  }

  private parseBooleanLiteral(): t.BooleanLiteral {
    if (!this.tokenIs('Boolean')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;

    this.advance();

    return new nodes.BooleanLiteral(
      value === 'true',
      lineStart,
      colStart,
      lineEnd,
      colEnd
    );
  }

  private parseNumericLiteral(): t.NumericLiteral {
    if (!this.tokenIs('Number')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    const style = value.indexOf('.') > -1 ? 'float' : 'integer';
    const numericValue = Number(value);

    this.advance();

    return new nodes.NumericLiteral(
      style,
      numericValue,
      value,
      lineStart,
      colStart,
      lineEnd,
      colEnd
    );
  }

  parseStringLiteral(): t.StringLiteral {
    if (!this.tokenIs('String')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;

    this.advance();

    return new nodes.StringLiteral(value, lineStart, colStart, lineEnd, colEnd);
  }

  parseExpression(): t.Expression {
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

  parseModule(): t.Module {
    const definitions: t.Definition[] = [];
    const body: t.Expression[] = [];

    let node;

    while ((node = this.parseExpression())) {
      body.push(node);
    }

    const bodyLineEnd = body.length ? body[body.length - 1].lineEnd : 0;
    const bodyColEnd = body.length ? body[body.length - 1].colEnd : 0;

    return new nodes.Module(definitions, body, 1, 0, bodyLineEnd, bodyColEnd);
  }
}

export function parseExpression(tokens: t.Token[], source: string): t.Expression {
  return new Parser(tokens, source).parseExpression();
}

export function parseModule(tokens: t.Token[], source: string): t.Module {
  return new Parser(tokens, source).parseModule();
}
