import { Visitor } from './visit';

type TokenKind =
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

type NodeKind =
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

interface Token {
  kind: TokenKind;
  value?: string;
  lineStart: number;
  lineEnd: number;
  colStart: number;
  colEnd: number;
}

interface AstNode {
  kind: NodeKind;
  type: TypeExpression | null;
  lineStart: number;
  lineEnd: number;
  colStart: number;
  colEnd: number;
  accept(visitor: Visitor): void;
}

type Literal =
  | BooleanLiteral
  | CharLiteral
  | NumericLiteral
  | InterpolatedStringLiteral
  | StringLiteral;

type Expression =
  | AssignmentExpression
  | BinaryExpression
  | Block
  | CallExpression
  | Identifier
  | Literal;

type Definition = TypeDefinition;

interface AssignmentExpression extends AstNode {
  kind: 'AssignmentExpression';
  leftSide: Identifier;
  rightSide: Expression;
}

interface BinaryExpression extends AstNode {
  kind: 'BinaryExpression';
  operator: Operator;
  leftSide: Expression;
  rightSide: Expression;
}

interface Block extends AstNode {
  kind: 'Block';
  parameters: Identifier[];
  body: Expression;
}

interface BooleanLiteral extends AstNode {
  kind: 'BooleanLiteral';
  value: boolean;
}

interface CallExpression extends AstNode {
  kind: 'CallExpression';
  receiver: Identifier | null;
  methodNameParts: Identifier[];
  arguments: Expression[][];
}

interface CharLiteral extends AstNode {
  kind: 'CharLiteral';
  value: string;
}

interface DictEntry extends AstNode {
  kind: 'DictEntry';
  key: StringLiteral;
  value: Expression;
}

interface DictExpression extends AstNode {
  kind: 'DictExpression';
  entries: DictEntry[];
}

interface ListExpression extends AstNode {
  kind: 'ListExpression';
  elements: Expression[];
}

interface Module extends AstNode {
  kind: 'Module';
  // imports: Import[];
  definitions: Definition[];
  body: Expression[];
}

interface NumericLiteral extends AstNode {
  kind: 'NumericLiteral';
  style: 'integer' | 'float';
  value: number;
  rawValue: string;
}

interface Identifier extends AstNode {
  kind: 'Identifier';
  name: string;
}

interface InterpolatedStringLiteral extends AstNode {
  kind: 'InterpolatedStringLiteral';
  literals: StringLiteral[];
  interpolations: Expression[];
}

interface MethodDefinition extends AstNode {
  kind: 'MethodDefinition';
  exported: boolean;
  methodNameParts: Identifier[];
  // selfType: TypeExpression;
  // paramTypes: TypeExpression;
  // returnType: TypeExpression;
}

interface Operator extends AstNode {
  kind: 'Operator';
  name: string;
}

interface StringLiteral extends AstNode {
  kind: 'StringLiteral';
  value: string;
}

interface TypeDefinition extends AstNode {
  kind: 'TypeDefinition';
  exported: boolean;
  name: Identifier;
  value: TypeExpression;
}

type TypeExpression = TypeIdentifier;

interface TypeIdentifier extends AstNode {
  kind: 'TypeIdentifier';
  name: string;
  typeParameters: TypeIdentifier[] | null;
}
