import { IVisitor } from './visitor';

export interface ISourceLocation {
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
}

export type SyntaxNodeKind =
  | 'ArrayExpression'
  | 'Assignment'
  | 'Block'
  | 'BooleanLiteral'
  | 'Call'
  | 'Identifier'
  | 'StringLiteral'
  | 'NumberLiteral'
  | 'StringExpression'
  | 'MemberExpression'
  | 'Range'
  | 'Module'
  | 'ModuleSpecifier';

export interface ISyntaxNode {
  kind: SyntaxNodeKind;
  comments: string[];
  location: ISourceLocation;
  accept(visitor: IVisitor): void;
}

export interface IExpressionNode extends ISyntaxNode {
  type: null | string;
}
