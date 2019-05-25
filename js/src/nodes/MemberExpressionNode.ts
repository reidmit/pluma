import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IIdentifierNode } from './IdentifierNode';
import { IVisitor } from '../visitor';

export interface IMemberExpressionNode extends IExpressionNode {
  kind: 'MemberExpression';
  expression: IExpressionNode;
  member: IIdentifierNode;
}

export class MemberExpressionNode extends BaseExpressionNode implements IMemberExpressionNode {
  readonly kind = 'MemberExpression';
  readonly expression: IExpressionNode;
  readonly member: IIdentifierNode;

  constructor(expression: IExpressionNode, member: IIdentifierNode) {
    super();
    this.expression = expression;
    this.member = member;
  }

  accept(visitor: IVisitor) {
    this.expression.accept(visitor);
    this.member.accept(visitor);
    visitor.visitMemberExpression(this);
  }
}
