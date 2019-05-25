import { ISyntaxNode, SyntaxNodeKind, ISourceLocation } from '../types';
import { IVisitor } from '../visitor';

export abstract class BaseNode implements ISyntaxNode {
  abstract readonly kind: SyntaxNodeKind;
  readonly comments: string[] = [];
  readonly location: ISourceLocation = {
    lineStart: -1,
    colStart: -1,
    lineEnd: -1,
    colEnd: -1
  };

  withComments(comments: string[]) {
    this.comments.push(...comments);
    return this;
  }

  withLocation(
    lineStart: number,
    colStart: number,
    lineEnd: number = lineStart,
    colEnd: number = colStart
  ) {
    this.location.lineStart = lineStart;
    this.location.colStart = colStart;
    this.location.lineEnd = lineEnd;
    this.location.colEnd = colEnd;
    return this;
  }

  abstract accept(visitor: IVisitor): void;
}
