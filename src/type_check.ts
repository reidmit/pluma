import { ISyntaxNode } from './types';
import { BaseVisitor } from './visitor';
import { INumberLiteralNode } from './nodes/NumberLiteralNode';
import { IStringLiteralNode } from './nodes/StringLiteralNode';
import { IBooleanLiteralNode } from './nodes/BooleanLiteralNode';
import { IAssignmentNode } from './nodes/AssignmentNode';
import { IStringExpressionNode } from './nodes/StringExpressionNode';
import { IArrayExpressionNode } from './nodes/ArrayExpressionNode';

export function typeCheck(ast: ISyntaxNode) {
  const checker = new TypeChecker();
  ast.accept(checker);
  return ast;
}

class TypeChecker extends BaseVisitor {
  visitArrayExpression(node: IArrayExpressionNode) {
    const firstElement = node.elements[0];

    if (!firstElement) {
      node.type = '[]';
      return;
    }

    for (let i = 1; i < node.elements.length; i++) {
      if (node.elements[i].type !== firstElement.type) {
        throw `Array element ${i} has incorrect type: ${node.elements[i].type} (expected ${
          firstElement.type
        })`;
      }
    }

    node.type = `[${firstElement.type}]`;
  }

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
