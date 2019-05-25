import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IIdentifierNode extends IExpressionNode {
  kind: 'Identifier';
  value: string;
}

export class IdentifierNode extends BaseExpressionNode implements IIdentifierNode {
  readonly kind = 'Identifier';
  readonly value: string;

  constructor(value: string) {
    super();
    this.value = value;
  }

  accept(visitor: IVisitor) {
    visitor.visitIdentifier(this);
  }
}
