import { ISyntaxNode } from './types';
import { IArrayExpressionNode } from './nodes/ArrayExpressionNode';
import { INumberLiteralNode } from './nodes/NumberLiteralNode';
import { IAssignmentNode } from './nodes/AssignmentNode';
import { IBlockNode } from './nodes/BlockNode';
import { IBooleanLiteralNode } from './nodes/BooleanLiteralNode';
import { ICallNode } from './nodes/CallNode';
import { IIdentifierNode } from './nodes/IdentifierNode';
import { IMemberExpressionNode } from './nodes/MemberExpressionNode';
import { IModuleNode } from './nodes/ModuleNode';
import { IStringLiteralNode } from './nodes/StringLiteralNode';
import { IStringExpressionNode } from './nodes/StringExpressionNode';

export interface IVisitor {
  // visit(node: ISyntaxNode): void;

  visitArrayExpression(node: IArrayExpressionNode): void;

  visitAssignment(node: IAssignmentNode): void;

  visitBlock(node: IBlockNode): void;

  visitBooleanLiteral(node: IBooleanLiteralNode): void;

  visitCall(node: ICallNode): void;

  visitIdentifier(node: IIdentifierNode): void;

  visitMemberExpression(node: IMemberExpressionNode): void;

  visitModule(node: IModuleNode): void;

  visitNumberLiteral(node: INumberLiteralNode): void;

  visitStringExpression(node: IStringExpressionNode): void;

  visitStringLiteral(node: IStringLiteralNode): void;
}

export class BaseVisitor implements IVisitor {
  // visit(node: ISyntaxNode) {
  //   switch (node.kind) {
  //     case 'ArrayExpression':
  //       return this.visitArrayExpression(node as IArrayExpressionNode);
  //     case 'Assignment':
  //       return this.visitAssignment(node as IAssignmentNode);
  //     case 'Block':
  //       return this.visitBlock(node as IBlockNode);
  //     case 'BooleanLiteral':
  //       return this.visitBooleanLiteral(node as IBooleanLiteralNode);
  //     case 'Call':
  //       return this.visitCall(node as ICallNode);
  //     case 'Identifier':
  //       return this.visitIdentifier(node as IIdentifierNode);
  //     case 'MemberExpression':
  //       return this.visitMemberExpression(node as IMemberExpressionNode);
  //     case 'Module':
  //       return this.visitModule(node as IModuleNode);
  //     case 'NumberLiteral':
  //       return this.visitNumberLiteral(node as INumberLiteralNode);
  //     case 'StringExpression':
  //       return this.visitStringExpression(node as IStringExpressionNode);
  //     case 'StringLiteral':
  //       return this.visitStringLiteral(node as IStringLiteralNode);
  //   }
  // }

  visitArrayExpression(node: IArrayExpressionNode) {}

  visitAssignment(node: IAssignmentNode) {}

  visitBlock(node: IBlockNode) {}

  visitBooleanLiteral(node: IBooleanLiteralNode) {}

  visitCall(node: ICallNode) {}

  visitIdentifier(node: IIdentifierNode) {}

  visitMemberExpression(node: IMemberExpressionNode) {}

  visitModule(node: IModuleNode) {}

  visitNumberLiteral(node: INumberLiteralNode) {}

  visitStringExpression(node: IStringExpressionNode) {}

  visitStringLiteral(node: IStringLiteralNode) {}
}
