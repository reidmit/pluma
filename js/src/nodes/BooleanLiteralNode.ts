import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IBooleanLiteralNode extends IExpressionNode {
  kind: 'BooleanLiteral';
  value: string;
}

export class BooleanLiteralNode extends BaseExpressionNode implements IBooleanLiteralNode {
  readonly kind = 'BooleanLiteral';
  readonly value: string;

  constructor(value: string) {
    super();
    this.value = value;
  }

  accept(visitor: IVisitor) {
    visitor.visitBooleanLiteral(this);
  }
}
