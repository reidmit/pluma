import { IExpressionNode } from '../types';
import { IIdentifierNode } from './IdentifierNode';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IAssignmentNode extends IExpressionNode {
  kind: 'Assignment';
  leftSide: IIdentifierNode;
  rightSide: IExpressionNode;
  constant: boolean;
}

export class AssignmentNode extends BaseExpressionNode implements IAssignmentNode {
  readonly kind = 'Assignment';
  readonly leftSide: IIdentifierNode;
  readonly rightSide: IExpressionNode;
  readonly constant: boolean;

  constructor(leftSide: IIdentifierNode, rightSide: IExpressionNode, constant: boolean) {
    super();
    this.leftSide = leftSide;
    this.rightSide = rightSide;
    this.constant = constant;
  }

  accept(visitor: IVisitor) {
    this.leftSide.accept(visitor);
    this.rightSide.accept(visitor);
    visitor.visitAssignment(this);
  }
}
