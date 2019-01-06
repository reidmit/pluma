type TokenKind =
  | 'Arrow'
  | 'Boolean'
  | 'Colon'
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
  | 'File'
  | 'FloatLiteral'
  | 'Identifier'
  | 'IntLiteral'
  | 'InterpolatedStringLiteral'
  | 'MethodDefinition'
  | 'Operator'
  | 'StringLiteral'
  | 'TypeDefinition';

interface SourceLocation {
  lineStart: number;
  lineEnd: number;
  colStart: number;
  colEnd: number;
}

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
  type: null;
  location: SourceLocation;
}

type Literal =
  | BooleanLiteral
  | CharLiteral
  | FloatLiteral
  | IntLiteral
  | InterpolatedStringLiteral
  | StringLiteral;

type Expression = AssignmentExpression | BinaryExpression;

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

interface File extends AstNode {
  kind: 'File';
  // imports: Import[];
  definitions: Definition[];
  body: Expression[];
}

interface FloatLiteral extends AstNode {
  kind: 'FloatLiteral';
  value: number;
}

interface Identifier extends AstNode {
  kind: 'Identifier';
  name: string;
}

interface IntLiteral extends AstNode {
  kind: 'IntLiteral';
  value: number;
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
  // value: TypeExpression;
}
