import { nodeTypes } from './constants';

class BaseNode {
  constructor(lineStart, lineEnd) {
    this.lineStart = lineStart;
    this.lineEnd = lineEnd;
  }
}

export class ArrayNode extends BaseNode {
  constructor(lineStart, lineEnd, elements) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.ARRAY;
    this.elements = elements;
  }
}

export class AssignmentNode extends BaseNode {
  constructor(lineStart, lineEnd, leftSide, rightSide) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.ASSIGNMENT;
    this.leftSide = leftSide;
    this.rightSide = rightSide;
  }
}

export class BooleanNode extends BaseNode {
  constructor(lineStart, lineEnd, value) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.BOOLEAN;
    this.value = value;
  }
}

export class CallNode extends BaseNode {
  constructor(lineStart, lineEnd, callee, arg) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.CALL;
    this.callee = callee;
    this.arg = arg;
  }
}

export class ConditionalNode extends BaseNode {
  constructor(lineStart, lineEnd, predicate, thenCase, elseCase) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.CONDITIONAL;
    this.predicate = predicate;
    this.thenCase = thenCase;
    this.elseCase = elseCase;
  }
}

export class FunctionNode extends BaseNode {
  constructor(lineStart, lineEnd, parameter, body) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.FUNCTION;
    this.parameter = parameter;
    this.body = body;
  }
}

export class IdentifierNode extends BaseNode {
  constructor(lineStart, lineEnd, value, isGetter, isSetter) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.IDENTIFIER;
    this.value = value;
    this.isGetter = isGetter;
    this.isSetter = isSetter;
  }
}

export class InterpolatedStringNode extends BaseNode {
  constructor(lineStart, lineEnd, literals, expressions) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.INTERPOLATED_STRING;
    this.literals = literals;
    this.expressions = expressions;
  }
}

export class ModuleNode extends BaseNode {
  constructor(lineStart, lineEnd, body) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.MODULE;
    this.body = body;
  }
}
export class NumberNode extends BaseNode {
  constructor(lineStart, lineEnd, value) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.NUMBER;
    this.value = value;
  }
}

export class StringNode extends BaseNode {
  constructor(lineStart, lineEnd, value) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.STRING;
    this.value = value;
  }
}

export class MemberExpressionNode extends BaseNode {
  constructor(lineStart, lineEnd, identifiers) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.MEMBER_EXPRESSION;
    this.identifiers = identifiers;
  }
}

export class ObjectNode extends BaseNode {
  constructor(lineStart, lineEnd, properties) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.OBJECT;
    this.properties = properties;
  }
}

export class ObjectPropertyNode extends BaseNode {
  constructor(lineStart, lineEnd, key, value) {
    super(lineStart, lineEnd);

    this.type = nodeTypes.OBJECT_PROPERTY;
    this.key = key;
    this.value = value;
  }
}
