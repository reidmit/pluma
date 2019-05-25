import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

type Radix = 10 | 2 | 8 | 16;

export interface INumberLiteralNode extends IExpressionNode {
  kind: 'NumberLiteral';
  value: string;
  radix: Radix;
}

export class NumberLiteralNode extends BaseExpressionNode implements INumberLiteralNode {
  readonly kind = 'NumberLiteral';
  readonly value: string;
  readonly radix: Radix;

  constructor(value: string, radix: Radix) {
    super();
    this.value = value;
    this.radix = radix;
  }

  accept(visitor: IVisitor) {
    visitor.visitNumberLiteral(this);
  }
}
