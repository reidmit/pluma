import { Token, TokenKind } from './tokens';
import {
  ModuleNode,
  ExpressionNode,
  AssignmentExpressionNode,
  IdentifierNode,
  CallExpressionNode,
  BlockExpressionNode,
  BooleanLiteralNode,
  StringExpressionNode,
  StringLiteralNode,
  ArrayExpressionNode,
  NumberLiteralNode
} from './nodes';
import { ParseError } from './errors';
import { tokenize } from './tokenize';
export { parse };

function parse(source: string): ModuleNode {
  return new Parser(tokenize(source), source).parseModule();
}

class Parser {
  private readonly tokens: Token[];
  private readonly source: string;
  private readonly tokenCount: number;
  private index: number;
  private token: Token;
  private comments: Map<number, string>;
  private eof: boolean;

  constructor(tokens: Token[], source: string) {
    this.tokens = tokens;
    this.source = source;
    this.tokenCount = this.tokens.length;
    this.index = 0;
    this.token = this.tokens[this.index];
    this.comments = new Map();
    this.eof = false;
  }

  private advance(amount: number = 1) {
    this.index += amount;
    this.token = this.tokens[this.index];
    if (this.index >= this.tokenCount) this.eof = true;
  }

  private tokenIs(kind: TokenKind, nextKind?: TokenKind) {
    if (this.eof) return false;
    if (this.token.kind !== kind) return false;

    if (nextKind) {
      const nextToken = this.tokens[this.index + 1];
      if (!nextToken) return false;
      if (nextToken.kind !== nextKind) return false;
    }

    return true;
  }

  private collectCommentsForLine(line: number): string[] {
    const comments: string[] = [];

    let lineComment;
    while ((lineComment = this.comments.get(line - 1))) {
      comments.push(lineComment);
      this.comments.delete(line);
      line--;
    }

    return comments.reverse();
  }

  private parseComments(): void {
    while (!this.eof && this.tokenIs('Comment')) {
      const { lineStart, value } = this.token;
      this.comments.set(lineStart, value);
      this.advance();
    }
  }

  private parseParenthetical(): ExpressionNode | void {
    if (!this.tokenIs('(')) return;

    const { lineStart, colStart } = this.token;
    this.advance();
    const expr = this.parseExpression();

    if (!this.tokenIs(')')) {
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

  private parseAssignmentExpression(): AssignmentExpressionNode | void {
    if (!(this.tokenIs('Identifier', '=') || this.tokenIs('Identifier', ':='))) {
      return;
    }

    const id = this.parseIdentifier();
    if (!id) return;

    const constant = this.tokenIs('=');
    this.advance();

    const expr = this.parseExpression();

    if (!expr) {
      throw new ParseError(
        id.lineStart,
        id.colEnd,
        'Expected expression after = in assignment'
      );
    }

    return {
      kind: 'AssignmentExpression',
      left: id,
      right: expr,
      constant,
      comments: this.collectCommentsForLine(id.lineStart),
      lineStart: id.lineStart,
      colStart: id.colStart,
      lineEnd: expr.lineEnd,
      colEnd: expr.lineEnd,
      typeAnnotation: null,
      type: null
    };
  }

  private parseIdentifier(): IdentifierNode | void {
    if (!this.tokenIs('Identifier')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    this.advance();

    return {
      kind: 'Identifier',
      comments: this.collectCommentsForLine(lineStart),
      value,
      lineStart,
      colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  private parseCallOrIdentifier(): CallExpressionNode | IdentifierNode | void {
    if (!this.tokenIs('Identifier')) return;

    const id = this.parseIdentifier();
    if (!id) return;

    const isCall = this.tokenIs('(') && this.token.lineStart === id.lineEnd;

    if (!isCall) return id;
    this.advance();

    const args: ExpressionNode[] = [];

    let arg;
    while ((arg = this.parseExpression())) {
      args.push(arg);

      if (this.tokenIs(',')) {
        this.advance();
        continue;
      }

      break;
    }

    if (!this.tokenIs(')')) {
      throw new ParseError(
        this.token.lineStart,
        this.token.colStart,
        'Expected ) after arguments'
      );
    }

    const { lineEnd, colEnd } = this.token;
    this.advance();

    return {
      kind: 'CallExpression',
      id,
      args,
      comments: this.collectCommentsForLine(id.lineStart),
      lineStart: id.lineStart,
      colStart: id.colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  private parseBlock(): BlockExpressionNode | void {
    if (!this.tokenIs('{')) return;

    const { lineStart, colStart } = this.token;
    this.advance();

    const paramNames = Object.create(null);
    const params: IdentifierNode[] = [];
    const firstExpression = this.parseExpression();

    const body: ExpressionNode[] = [];

    if (firstExpression) {
      if (
        firstExpression.kind === 'Identifier' &&
        (this.tokenIs(',') || this.tokenIs('=>'))
      ) {
        paramNames[firstExpression.value] = true;
        params.push(firstExpression);

        while (this.tokenIs(',')) {
          this.advance();
          const parameter = this.parseIdentifier();
          if (!parameter) {
            throw new ParseError(-1, -1, 'Expected a parameter name after , in block');
          }

          if (paramNames[parameter.value]) {
            throw new ParseError(
              parameter.lineStart,
              parameter.colStart,
              `Multiple parameters in block with name '${parameter.value}'`
            );
          }

          paramNames[parameter.value] = true;
          params.push(parameter);
        }

        if (!this.tokenIs('=>')) {
          throw new ParseError(
            this.token.lineStart,
            this.token.lineEnd,
            `Expected a => after block parameters, but found ${this.token.kind}`
          );
        }

        this.advance();
      } else {
        body.push(firstExpression);
      }
    }

    let bodyExpr;
    while ((bodyExpr = this.parseExpression())) {
      body.push(bodyExpr);
    }

    if (params.length && !body.length) {
      throw new ParseError(-1, -1, 'Expected an expression after => in block');
    }

    if (!this.tokenIs('}')) {
      throw new ParseError(
        this.token.lineStart,
        this.token.lineEnd,
        `Expected a } to close block, but found ${this.token.kind}`
      );
    }

    const { lineEnd, colEnd } = this.token;
    this.advance();

    return {
      kind: 'BlockExpression',
      params,
      body,
      comments: this.collectCommentsForLine(lineStart),
      lineStart,
      colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  private parseBooleanLiteral(): BooleanLiteralNode | void {
    if (!this.tokenIs('Boolean')) return;
    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    this.advance();

    return {
      kind: 'BooleanLiteral',
      comments: this.collectCommentsForLine(lineStart),
      value,
      lineStart,
      colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  private parseNumberLiteral(): NumberLiteralNode | void {
    if (!this.tokenIs('Number')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    const radix = /^0b/.test(value)
      ? 2
      : /^0o/.test(value)
      ? 8
      : /^0x/.test(value)
      ? 16
      : 10;

    this.advance();

    return {
      kind: 'NumberLiteral',
      value,
      radix,
      comments: this.collectCommentsForLine(lineStart),
      lineStart,
      colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  private parseStringLiteral(): StringLiteralNode | void {
    if (!this.tokenIs('String')) return;
    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    this.advance();

    return {
      kind: 'StringLiteral',
      comments: this.collectCommentsForLine(lineStart),
      value,
      lineStart,
      colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  private parseStringExpression(): StringExpressionNode | void {
    const firstPart = this.parseStringLiteral();
    if (!firstPart) return;

    const expr: StringExpressionNode = {
      kind: 'StringExpression',
      parts: [firstPart],
      comments: this.collectCommentsForLine(firstPart.lineStart),
      lineStart: firstPart.lineStart,
      colStart: firstPart.colStart,
      lineEnd: firstPart.lineEnd,
      colEnd: firstPart.colEnd,
      type: null
    };

    while (this.tokenIs('InterpolationStart')) {
      const startToken = this.token;
      this.advance();

      const innerExpr = this.parseExpression();

      if (!innerExpr) {
        throw new ParseError(
          startToken.lineStart,
          startToken.colStart,
          'Expected an expression after $( in string interpolation.'
        );
      }

      expr.parts.push(innerExpr);

      if (!this.tokenIs('InterpolationEnd')) {
        throw new ParseError(
          startToken.lineStart,
          startToken.lineStart,
          'Expected a closing ) after string interpolation.'
        );
      }

      this.advance();
      const nextStringLiteral = this.parseStringLiteral();

      if (nextStringLiteral) {
        expr.parts.push(nextStringLiteral);
      }
    }

    const lastPart = expr.parts[expr.parts.length - 1];
    expr.lineEnd = lastPart.lineEnd;
    expr.colEnd = lastPart.colEnd;

    return expr;
  }

  private parseArrayExpression(): ArrayExpressionNode | void {
    if (!this.tokenIs('[')) return;

    const { lineStart, colStart } = this.token;
    this.advance();

    const elements = [];

    let expr;
    while ((expr = this.parseExpression())) {
      elements.push(expr);

      if (this.tokenIs(',')) {
        this.advance();
        continue;
      }

      break;
    }

    if (!this.tokenIs(']')) {
      throw new ParseError(
        this.token.lineStart,
        this.token.colStart,
        'Expected closing ]'
      );
    }

    const { lineEnd, colEnd } = this.token;
    this.advance();

    return {
      kind: 'ArrayExpression',
      elements,
      comments: this.collectCommentsForLine(lineStart),
      lineStart,
      colStart,
      lineEnd,
      colEnd,
      type: null
    };
  }

  parseExpression(): ExpressionNode | void {
    if (this.eof) return;

    this.parseComments();
    if (this.eof) return;

    return (
      this.parseAssignmentExpression() ||
      this.parseParenthetical() ||
      this.parseBlock() ||
      this.parseArrayExpression() ||
      this.parseCallOrIdentifier() ||
      this.parseBooleanLiteral() ||
      this.parseNumberLiteral() ||
      this.parseStringExpression()
    );
  }

  parseModule(): ModuleNode {
    const body: ExpressionNode[] = [];

    let node;
    while ((node = this.parseExpression())) {
      body.push(node);
    }

    const bodyLineEnd = body.length ? body[body.length - 1].lineEnd : 0;
    const bodyColEnd = body.length ? body[body.length - 1].colEnd : 0;

    return {
      kind: 'Module',
      body,
      comments: [],
      lineStart: 1,
      colStart: 0,
      lineEnd: bodyLineEnd,
      colEnd: bodyColEnd
    };
  }
}
