class AssignmentExpression extends AstNode {
  kind: 'AssignmentExpression';
  leftSide: Identifier;
  rightSide: Expression;
}

class BinaryExpression extends AstNode {
  kind: 'BinaryExpression';
  operator: Operator;
  leftSide: Expression;
  rightSide: Expression;
}

class Block extends AstNode {
  kind: 'Block';
  parameters: Identifier[];
  body: Expression;
}

class BooleanLiteral extends AstNode {
  kind: 'BooleanLiteral';
  value: boolean;
}

class CallExpression extends AstNode {
  kind: 'CallExpression';
  receiver: Identifier | null;
  methodNameParts: Identifier[];
  arguments: Expression[][];
}

class CharLiteral extends AstNode {
  kind: 'CharLiteral';
  value: string;
}

class DictEntry extends AstNode {
  kind: 'DictEntry';
  key: StringLiteral;
  value: Expression;
}

class DictExpression extends AstNode {
  kind: 'DictExpression';
  entries: DictEntry[];
}

class ListExpression extends AstNode {
  kind: 'ListExpression';
  elements: Expression[];
}

class Module extends AstNode {
  kind: 'Module';
  // imports: Import[];
  definitions: Definition[];
  body: Expression[];
}

class NumericLiteral extends AstNode {
  kind: 'NumericLiteral';
  style: 'integer' | 'float';
  value: number;
  rawValue: string;
}

class Identifier extends AstNode {
  kind: 'Identifier';
  name: string;
}

class InterpolatedStringLiteral extends AstNode {
  kind: 'InterpolatedStringLiteral';
  literals: StringLiteral[];
  interpolations: Expression[];
}

class MethodDefinition extends AstNode {
  kind: 'MethodDefinition';
  exported: boolean;
  methodNameParts: Identifier[];
  // selfType: TypeExpression;
  // paramTypes: TypeExpression;
  // returnType: TypeExpression;
}

class Operator extends AstNode {
  kind: 'Operator';
  name: string;
}

class StringLiteral extends AstNode {
  kind: 'StringLiteral';
  value: string;
}

class TypeDefinition extends AstNode {
  kind: 'TypeDefinition';
  exported: boolean;
  name: Identifier;
  value: TypeExpression;
}

type TypeExpression = TypeIdentifier;

class TypeIdentifier extends AstNode {
  kind: 'TypeIdentifier';
  name: string;
  typeParameters: TypeIdentifier[] | null;
}
