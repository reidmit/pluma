import { Visitor } from './visit';

export type TokenKind =
  | 'Arrow'
  | 'Boolean'
  | 'Char'
  | 'Colon'
  | 'ColonEquals'
  | 'Comma'
  | 'Comment'
  | 'Dot'
  | 'DoubleArrow'
  | 'Equals'
  | 'Identifier'
  | 'InterpolationEnd'
  | 'InterpolationStart'
  | 'LeftBrace'
  | 'LeftBracket'
  | 'LeftParen'
  | 'Operator'
  | 'Number'
  | 'RightBrace'
  | 'RightBracket'
  | 'RightParen'
  | 'String';

export type NodeKind =
  | 'AssignmentExpression'
  | 'BinaryExpression'
  | 'Block'
  | 'BooleanLiteral'
  | 'CallExpression'
  | 'CharLiteral'
  | 'DictEntry'
  | 'DictExpression'
  | 'Identifier'
  | 'InterpolatedStringLiteral'
  | 'ListExpression'
  | 'MethodDefinition'
  | 'Module'
  | 'NumericLiteral'
  | 'Operator'
  | 'StringLiteral'
  | 'TypeDefinition'
  | 'TypeExpression'
  | 'TypeIdentifier';

export interface Token {
  kind: TokenKind;
  value?: string;
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
}

export type Literal =
  | BooleanLiteral
  | CharLiteral
  | NumericLiteral
  | InterpolatedStringLiteral
  | StringLiteral;

export type Expression =
  | AssignmentExpression
  | BinaryExpression
  | Block
  | CallExpression
  | Identifier
  | Literal;

export type Definition = TypeDefinition | MethodDefinition;

export type TypeExpression = TypeIdentifier;

export interface AstNode {
  kind: NodeKind;
  type: TypeExpression | null;
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
  accept(visitor: Visitor): void;
}

export interface AssignmentExpression extends AstNode {
  kind: 'AssignmentExpression';
  leftSide: Identifier;
  rightSide: Expression;
}

export interface BinaryExpression extends AstNode {
  kind: 'BinaryExpression';
  operator: Operator;
  leftSide: Expression;
  rightSide: Expression;
}

export interface Block extends AstNode {
  kind: 'Block';
  parameters: Identifier[];
  body: Expression;
}

export interface BooleanLiteral extends AstNode {
  kind: 'BooleanLiteral';
  value: boolean;
}

export interface CallExpression extends AstNode {
  kind: 'CallExpression';
  receiver: Identifier | null;
  methodPartNames: Identifier[];
  methodPartArgs: Expression[][];
}

export interface CharLiteral extends AstNode {
  kind: 'CharLiteral';
  value: string;
}

export interface DictEntry extends AstNode {
  kind: 'DictEntry';
  key: StringLiteral;
  value: Expression;
}

export interface DictExpression extends AstNode {
  kind: 'DictExpression';
  entries: DictEntry[];
}

export interface ListExpression extends AstNode {
  kind: 'ListExpression';
  elements: Expression[];
}

export interface Module extends AstNode {
  kind: 'Module';
  // imports: Import[];
  definitions: Definition[];
  body: Expression[];
}

export interface NumericLiteral extends AstNode {
  kind: 'NumericLiteral';
  style: 'integer' | 'float';
  value: number;
  rawValue: string;
}

export interface Identifier extends AstNode {
  kind: 'Identifier';
  name: string;
}

export interface InterpolatedStringLiteral extends AstNode {
  kind: 'InterpolatedStringLiteral';
  literals: StringLiteral[];
  interpolations: Expression[];
}

export interface MethodDefinition extends AstNode {
  kind: 'MethodDefinition';
  exported: boolean;
  methodNameParts: Identifier[];
  // selfType: TypeExpression;
  // paramTypes: TypeExpression;
  // returnType: TypeExpression;
}

export interface Operator extends AstNode {
  kind: 'Operator';
  name: string;
}

export interface StringLiteral extends AstNode {
  kind: 'StringLiteral';
  value: string;
}

export interface TypeDefinition extends AstNode {
  kind: 'TypeDefinition';
  exported: boolean;
  name: Identifier;
  value: TypeExpression;
}

export interface TypeIdentifier extends AstNode {
  kind: 'TypeIdentifier';
  name: string;
  typeParameters: TypeIdentifier[] | null;
}
