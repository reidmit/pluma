export interface BaseNode {
  comments: string[];
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
}

export interface IdentifierNode extends BaseNode {
  kind: 'Identifier';
  value: string;
  type: null;
}

export interface StringLiteralNode extends BaseNode {
  kind: 'StringLiteral';
  value: string;
  type: null;
}

export interface NumberLiteralNode extends BaseNode {
  kind: 'NumberLiteral';
  value: string;
  radix: 10 | 2 | 8 | 16;
  type: null;
}

export interface BooleanLiteralNode extends BaseNode {
  kind: 'BooleanLiteral';
  value: string;
  type: null;
}

export interface StringExpressionNode extends BaseNode {
  kind: 'StringExpression';
  parts: ExpressionNode[];
  type: null;
}

export interface BlockExpressionNode extends BaseNode {
  kind: 'BlockExpression';
  params: IdentifierNode[];
  body: ExpressionNode[];
  type: null;
}

export interface AssignmentExpressionNode extends BaseNode {
  kind: 'AssignmentExpression';
  left: IdentifierNode;
  right: ExpressionNode;
  constant: boolean;
  typeAnnotation: null;
  type: null;
}

export interface CallExpressionNode extends BaseNode {
  kind: 'CallExpression';
  id: IdentifierNode;
  args: ExpressionNode[];
  type: null;
}

export interface ArrayExpressionNode extends BaseNode {
  kind: 'ArrayExpression';
  elements: ExpressionNode[];
  type: null;
}

export interface ModuleNode extends BaseNode {
  kind: 'Module';
  body: ExpressionNode[];
}

export type ExpressionNode =
  | IdentifierNode
  | NumberLiteralNode
  | BooleanLiteralNode
  | StringLiteralNode
  | StringExpressionNode
  | BlockExpressionNode
  | AssignmentExpressionNode
  | CallExpressionNode
  | ArrayExpressionNode;
