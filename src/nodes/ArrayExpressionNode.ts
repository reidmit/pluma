import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IArrayExpressionNode extends IExpressionNode {
  kind: 'ArrayExpression';
  elements: IExpressionNode[];
}

export class ArrayExpressionNode extends BaseExpressionNode implements IArrayExpressionNode {
  readonly kind = 'ArrayExpression';
  readonly elements: IExpressionNode[];

  constructor(elements: IExpressionNode[]) {
    super();
    this.elements = elements;
  }

  accept(visitor: IVisitor) {
    for (const el of this.elements) {
      el.accept(visitor);
    }

    visitor.visitArrayExpression(this);
  }
}
