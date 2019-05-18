import { IVisitor } from './visitor';

export interface ISourceLocation {
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
}

export type SyntaxNodeKind =
  | 'Identifier'
  | 'StringLiteral'
  | 'NumberLiteral'
  | 'BooleanLiteral'
  | 'StringExpression'
  | 'ArrayExpression'
  | 'MemberExpression'
  | 'Block'
  | 'Assignment'
  | 'Call'
  | 'Module';

export interface ISyntaxNode {
  kind: SyntaxNodeKind;
  comments: string[];
  location: ISourceLocation;
  accept(visitor: IVisitor): void;
}

export interface IExpressionNode extends ISyntaxNode {
  type: null | string;
}
