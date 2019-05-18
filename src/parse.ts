import { Token, TokenKind } from './tokens';
import { ParseError } from './errors';
import { tokenize } from './tokenize';
import { IdentifierNode, IIdentifierNode } from './nodes/IdentifierNode';
import { IBooleanLiteralNode, BooleanLiteralNode } from './nodes/BooleanLiteralNode';
import { BlockNode, IBlockNode } from './nodes/BlockNode';
import { ISyntaxNode, IExpressionNode } from './types';
import { AssignmentNode, IAssignmentNode } from './nodes/AssignmentNode';
import { NumberLiteralNode, INumberLiteralNode } from './nodes/NumberLiteralNode';
import { StringLiteralNode, IStringLiteralNode } from './nodes/StringLiteralNode';
import { StringExpressionNode, IStringExpressionNode } from './nodes/StringExpressionNode';
import { ArrayExpressionNode, IArrayExpressionNode } from './nodes/ArrayExpressionNode';
import { CallNode, ICallNode } from './nodes/CallNode';
import { ModuleNode, IModuleNode } from './nodes/ModuleNode';
import { MemberExpressionNode } from './nodes/MemberExpressionNode';

export function parse(source: string): IModuleNode {
  return new Parser(tokenize(source), source).parseModule();
}

export class Parser {
  readonly tokens: Token[];
  readonly source: string;
  readonly fileName: string;
  private readonly tokenCount: number;
  private index: number;
  private token: Token;
  private comments: Map<number, string>;
  private eof: boolean;

  constructor(tokens: Token[], source: string) {
    this.tokens = tokens;
    this.source = source;
    this.fileName = '';
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

  private unexpectedToken() {
    if (this.eof) return;

    throw ParseError.fromToken(`Unexpected token: ${this.token.value}`, this.token, this);
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

  private parseParentheticalExpression(): IExpressionNode | void {
    if (!this.tokenIs('(')) return;

    const { lineStart, colStart } = this.token;
    this.advance();
    const expr = this.parseExpression();

    if (!this.tokenIs(')')) {
      throw ParseError.fromToken(
        `Expected a ) to match ( at line ${lineStart}, column ${colStart}, but found ${
          this.token.kind
        }`,
        this.token,
        this
      );
    }
    this.advance();

    return expr;
  }

  private parseAssignmentExpression(): IAssignmentNode | void {
    if (
      !(
        this.tokenIs('Identifier', '=') ||
        this.tokenIs('Identifier', ':=') ||
        this.tokenIs('Identifier', '::')
      )
    ) {
      return;
    }

    const id = this.parseIdentifier();
    if (!id) return;

    const constant = this.tokenIs('=');
    this.advance();

    const expr = this.parseExpression();

    if (!expr) {
      throw ParseError.fromSourceLocation(
        'Expected expression after = in assignment',
        id.location,
        this
      );
    }

    return new AssignmentNode(id, expr, constant)
      .withComments(this.collectCommentsForLine(id.location.lineStart))
      .withLocation(
        id.location.lineStart,
        id.location.colStart,
        expr.location.lineEnd,
        expr.location.colEnd
      );
  }

  private parseIdentifier(): IIdentifierNode | void {
    if (!this.tokenIs('Identifier')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    this.advance();

    return new IdentifierNode(value)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  private parseCallOrIdentifier(): ICallNode | IIdentifierNode | void {
    if (!this.tokenIs('Identifier')) return;

    const id = this.parseIdentifier();
    if (!id) return;

    const isCall = this.tokenIs('(') && this.token.lineStart === id.location.lineEnd;

    if (!isCall) return id;
    this.advance();

    const args: IExpressionNode[] = [];

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
      throw ParseError.fromToken('Expected ) after arguments', this.token, this);
    }

    const { lineEnd, colEnd } = this.token;
    this.advance();

    return new CallNode(id, args)
      .withComments(this.collectCommentsForLine(id.location.lineStart))
      .withLocation(id.location.lineStart, id.location.colStart, lineEnd, colEnd);
  }

  private parseBlockExpression(): IBlockNode | void {
    if (!this.tokenIs('{')) return;

    const { lineStart, colStart } = this.token;
    this.advance();

    const paramNames = Object.create(null);
    const params: IIdentifierNode[] = [];
    const firstExpression = this.parseExpression();

    const body: IExpressionNode[] = [];

    if (firstExpression) {
      if (firstExpression.kind === 'Identifier' && (this.tokenIs(',') || this.tokenIs('=>'))) {
        const firstParam = firstExpression as IIdentifierNode;
        paramNames[firstParam.value] = true;
        params.push(firstParam);

        while (this.tokenIs(',')) {
          this.advance();
          const paramToken = this.token;
          const parameter = this.parseIdentifier();

          if (!parameter) {
            throw ParseError.fromToken(
              'Expected a parameter name after , in block',
              paramToken,
              this
            );
          }

          if (paramNames[parameter.value]) {
            throw ParseError.fromSourceLocation(
              `Multiple parameters in block with name '${parameter.value}'`,
              parameter.location,
              this
            );
          }

          paramNames[parameter.value] = true;
          params.push(parameter);
        }

        if (!this.tokenIs('=>')) {
          throw ParseError.fromToken(
            `Expected a => after block parameters, but found ${this.token.kind}`,
            this.token,
            this
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
      throw ParseError.fromToken('Expected an expression after => in block', this.token, this);
    }

    if (!this.tokenIs('}')) {
      throw ParseError.fromToken(
        `Expected a } to close block, but found ${this.token.kind}`,
        this.token,
        this
      );
    }

    const { lineEnd, colEnd } = this.token;
    this.advance();

    return new BlockNode(params, body)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  private parseBooleanLiteral(): IBooleanLiteralNode | void {
    if (!this.tokenIs('Boolean')) return;
    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    this.advance();

    return new BooleanLiteralNode(value)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  private parseNumberLiteral(): INumberLiteralNode | void {
    if (!this.tokenIs('Number')) return;

    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    const radix = /^0b/.test(value) ? 2 : /^0o/.test(value) ? 8 : /^0x/.test(value) ? 16 : 10;

    this.advance();

    return new NumberLiteralNode(value, radix)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  private parseStringLiteral(): IStringLiteralNode | void {
    if (!this.tokenIs('String')) return;
    const { value, lineStart, lineEnd, colStart, colEnd } = this.token;
    this.advance();

    return new StringLiteralNode(value)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  private parseStringExpression(): IStringExpressionNode | void {
    const firstPart = this.parseStringLiteral();
    if (!firstPart) return;

    const { lineStart, colStart } = firstPart.location;
    const parts: IExpressionNode[] = [firstPart];

    while (this.tokenIs('InterpolationStart')) {
      const startToken = this.token;
      this.advance();

      const innerExpr = this.parseExpression();

      if (!innerExpr) {
        throw ParseError.fromToken(
          'Expected an expression after $( in string interpolation.',
          startToken,
          this
        );
      }

      parts.push(
        new CallNode(new IdentifierNode('toString'), [innerExpr]).withLocation(
          innerExpr.location.lineStart,
          innerExpr.location.colStart,
          innerExpr.location.lineEnd,
          innerExpr.location.colEnd
        )
      );

      if (!this.tokenIs('InterpolationEnd')) {
        throw ParseError.fromToken(
          'Expected a closing ) after string interpolation.',
          startToken,
          this
        );
      }

      this.advance();
      const nextStringLiteral = this.parseStringLiteral();

      if (nextStringLiteral) {
        parts.push(nextStringLiteral);
      }
    }

    const lastPart = parts[parts.length - 1];
    const { lineEnd, colEnd } = lastPart.location;

    return new StringExpressionNode(parts)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  private parseArrayExpression(): IArrayExpressionNode | void {
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
      throw ParseError.fromToken('Expected closing ]', this.token, this);
    }

    const { lineEnd, colEnd } = this.token;
    this.advance();

    return new ArrayExpressionNode(elements)
      .withComments(this.collectCommentsForLine(lineStart))
      .withLocation(lineStart, colStart, lineEnd, colEnd);
  }

  parseExpression(): IExpressionNode | void {
    this.parseComments();
    if (this.eof) return;

    let expr =
      this.parseAssignmentExpression() ||
      this.parseParentheticalExpression() ||
      this.parseBlockExpression() ||
      this.parseArrayExpression() ||
      this.parseCallOrIdentifier() ||
      this.parseBooleanLiteral() ||
      this.parseNumberLiteral() ||
      this.parseStringExpression();

    while (expr && this.tokenIs('.', 'Identifier')) {
      this.advance();
      const member = this.parseIdentifier() as IIdentifierNode;
      const { lineStart, colStart } = expr.location;
      const { lineEnd, colEnd } = member.location;
      expr = new MemberExpressionNode(expr, member)
        .withComments(this.collectCommentsForLine(expr.location.lineStart))
        .withLocation(lineStart, colStart, lineEnd, colEnd);
    }

    return expr;
  }

  parseModule(): IModuleNode {
    const body: ISyntaxNode[] = [];

    let node;
    while ((node = this.parseExpression())) {
      body.push(node);
    }

    if (!this.eof) this.unexpectedToken();

    const lastNode = body[body.length - 1];
    const bodyLineEnd = lastNode ? lastNode.location.lineEnd : 1;
    const bodyColEnd = lastNode ? lastNode.location.colEnd : 0;

    return new ModuleNode(body).withLocation(1, 0, bodyLineEnd, bodyColEnd);
  }
}
