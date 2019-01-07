import * as t from './types';
import { Visitor } from './visit';

class BaseNode {
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
  type: null;

  constructor(lineStart: number, colStart: number, lineEnd: number, colEnd: number) {
    this.lineStart = lineStart;
    this.colStart = colStart;
    this.lineEnd = lineEnd;
    this.colEnd = colEnd;
    this.type = null;
  }
}

export class AssignmentExpression extends BaseNode implements t.AstNode {
  kind: 'AssignmentExpression' = 'AssignmentExpression';
  leftSide: t.Identifier;
  rightSide: t.Expression;

  constructor(
    leftSide: t.Identifier,
    rightSide: t.Expression,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.leftSide = leftSide;
    this.rightSide = rightSide;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
    visitor.visit(this.leftSide);
    visitor.visit(this.rightSide);
  }
}

export class BinaryExpression extends BaseNode implements t.BinaryExpression {
  kind: 'BinaryExpression' = 'BinaryExpression';
  operator: t.Operator;
  leftSide: t.Expression;
  rightSide: t.Expression;

  constructor(
    operator: t.Operator,
    leftSide: t.Identifier,
    rightSide: t.Expression,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.operator = operator;
    this.leftSide = leftSide;
    this.rightSide = rightSide;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class Block extends BaseNode implements t.Block {
  kind: 'Block' = 'Block';
  parameters: t.Identifier[];
  body: t.Expression;

  constructor(
    parameters: t.Identifier[],
    body: t.Expression,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.parameters = parameters;
    this.body = body;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class BooleanLiteral extends BaseNode implements t.BooleanLiteral {
  kind: 'BooleanLiteral' = 'BooleanLiteral';
  value: boolean;

  constructor(
    value: boolean,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.value = value;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class CallExpression extends BaseNode implements t.CallExpression {
  kind: 'CallExpression' = 'CallExpression';
  receiver: t.Identifier | null;
  methodPartNames: t.Identifier[];
  methodPartArgs: t.Expression[][];

  constructor(
    receiver: t.Identifier | null,
    methodPartNames: t.Identifier[],
    methodPartArgs: t.Expression[][],
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.receiver = receiver;
    this.methodPartNames = methodPartNames;
    this.methodPartArgs = methodPartArgs;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class CharLiteral extends BaseNode implements t.CharLiteral {
  kind: 'CharLiteral' = 'CharLiteral';
  value: string;

  constructor(
    value: string,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.value = value;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class DictEntry extends BaseNode implements t.DictEntry {
  kind: 'DictEntry' = 'DictEntry';
  key: StringLiteral;
  value: t.Expression;

  constructor(
    key: StringLiteral,
    value: t.Expression,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.key = key;
    this.value = value;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class DictExpression extends BaseNode implements t.DictExpression {
  kind: 'DictExpression' = 'DictExpression';
  entries: t.DictEntry[];

  constructor(
    entries: t.DictEntry[],
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.entries = entries;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class ListExpression extends BaseNode implements t.ListExpression {
  kind: 'ListExpression' = 'ListExpression';
  elements: t.Expression[];

  constructor(
    elements: t.Expression[],
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.elements = elements;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class Module extends BaseNode implements t.Module {
  kind: 'Module' = 'Module';
  // imports: Import[];
  definitions: t.Definition[];
  body: t.Expression[];

  constructor(
    definitions: t.Definition[],
    body: t.Expression[],
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.definitions = definitions;
    this.body = body;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class NumericLiteral extends BaseNode implements t.NumericLiteral {
  kind: 'NumericLiteral' = 'NumericLiteral';
  style: 'integer' | 'float';
  value: number;
  rawValue: string;

  constructor(
    style: 'integer' | 'float',
    value: number,
    rawValue: string,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.style = style;
    this.value = value;
    this.rawValue = rawValue;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class Identifier extends BaseNode implements t.Identifier {
  kind: 'Identifier' = 'Identifier';
  name: string;

  constructor(
    name: string,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.name = name;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class InterpolatedStringLiteral extends BaseNode
  implements t.InterpolatedStringLiteral {
  kind: 'InterpolatedStringLiteral' = 'InterpolatedStringLiteral';
  literals: StringLiteral[];
  interpolations: t.Expression[];

  constructor(
    literals: StringLiteral[],
    interpolations: t.Expression[],
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.literals = literals;
    this.interpolations = interpolations;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class MethodDefinition extends BaseNode implements t.MethodDefinition {
  kind: 'MethodDefinition' = 'MethodDefinition';
  exported: boolean;
  methodNameParts: t.Identifier[];
  // selfType: TypeExpression;
  // paramTypes: TypeExpression;
  // returnType: TypeExpression;

  constructor(
    exported: boolean,
    methodNameParts: t.Identifier[],
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.exported = exported;
    this.methodNameParts = methodNameParts;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class Operator extends BaseNode implements t.Operator {
  kind: 'Operator' = 'Operator';
  name: string;

  constructor(
    name: string,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.name = name;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class StringLiteral extends BaseNode implements t.StringLiteral {
  kind: 'StringLiteral' = 'StringLiteral';
  value: string;

  constructor(
    value: string,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.value = value;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class TypeDefinition extends BaseNode implements t.TypeDefinition {
  kind: 'TypeDefinition' = 'TypeDefinition';
  exported: boolean;
  name: t.Identifier;
  value: t.TypeExpression;

  constructor(
    exported: boolean,
    name: t.Identifier,
    value: t.TypeExpression,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.exported = exported;
    this.name = name;
    this.value = value;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}

export class TypeIdentifier extends BaseNode implements t.TypeIdentifier {
  kind: 'TypeIdentifier' = 'TypeIdentifier';
  name: string;
  typeParameters: t.TypeIdentifier[] | null;

  constructor(
    name: string,
    typeParameters: t.TypeIdentifier[] | null,
    lineStart: number,
    colStart: number,
    lineEnd: number,
    colEnd: number
  ) {
    super(lineStart, colStart, lineEnd, colEnd);
    this.name = name;
    this.typeParameters = typeParameters;
  }

  accept(visitor: Visitor) {
    visitor.visit(this);
  }
}
