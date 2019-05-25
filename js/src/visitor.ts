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
import { IModuleSpecifierNode } from './nodes/ModuleSpecifierNode';

export interface IVisitor {
  visitArrayExpression(node: IArrayExpressionNode): void;
  visitAssignment(node: IAssignmentNode): void;
  visitBlock(node: IBlockNode): void;
  visitBooleanLiteral(node: IBooleanLiteralNode): void;
  visitCall(node: ICallNode): void;
  visitIdentifier(node: IIdentifierNode): void;
  visitMemberExpression(node: IMemberExpressionNode): void;
  visitModule(node: IModuleNode): void;
  visitModuleSpecifier(node: IModuleSpecifierNode): void;
  visitNumberLiteral(node: INumberLiteralNode): void;
  visitStringExpression(node: IStringExpressionNode): void;
  visitStringLiteral(node: IStringLiteralNode): void;
}

export class BaseVisitor implements IVisitor {
  visitArrayExpression(node: IArrayExpressionNode) {}
  visitAssignment(node: IAssignmentNode) {}
  visitBlock(node: IBlockNode) {}
  visitBooleanLiteral(node: IBooleanLiteralNode) {}
  visitCall(node: ICallNode) {}
  visitIdentifier(node: IIdentifierNode) {}
  visitMemberExpression(node: IMemberExpressionNode) {}
  visitModule(node: IModuleNode) {}
  visitModuleSpecifier(node: IModuleSpecifierNode) {}
  visitNumberLiteral(node: INumberLiteralNode) {}
  visitStringExpression(node: IStringExpressionNode) {}
  visitStringLiteral(node: IStringLiteralNode) {}
}
