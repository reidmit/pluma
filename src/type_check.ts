import { ISyntaxNode } from './types';
import { BaseVisitor } from './visitor';
import { INumberLiteralNode } from './nodes/NumberLiteralNode';
import { IStringLiteralNode } from './nodes/StringLiteralNode';
import { IBooleanLiteralNode } from './nodes/BooleanLiteralNode';
import { IAssignmentNode } from './nodes/AssignmentNode';
import { IStringExpressionNode } from './nodes/StringExpressionNode';

export function typeCheck(ast: ISyntaxNode) {
  const checker = new TypeChecker();
  ast.accept(checker);
  return ast;
}

class TypeChecker extends BaseVisitor {
  visitAssignment(node: IAssignmentNode) {
    node.type = node.rightSide.type;
  }

  visitBooleanLiteral(node: IBooleanLiteralNode) {
    node.type = 'Boolean';
  }

  visitNumberLiteral(node: INumberLiteralNode) {
    node.type = 'Number';
  }

  visitStringExpression(node: IStringExpressionNode) {
    node.type = 'String';
  }

  visitStringLiteral(node: IStringLiteralNode) {
    node.type = 'String';
  }
}
