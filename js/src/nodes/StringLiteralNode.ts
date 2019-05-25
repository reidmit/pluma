import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IStringLiteralNode extends IExpressionNode {
  kind: 'StringLiteral';
  value: string;
}

export class StringLiteralNode extends BaseExpressionNode implements IStringLiteralNode {
  readonly kind = 'StringLiteral';
  readonly value: string;

  constructor(value: string) {
    super();
    this.value = value;
  }

  accept(visitor: IVisitor) {
    visitor.visitStringLiteral(this);
  }
}
