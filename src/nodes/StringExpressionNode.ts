import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IStringExpressionNode extends IExpressionNode {
  kind: 'StringExpression';
  parts: IExpressionNode[];
}

export class StringExpressionNode extends BaseExpressionNode implements IStringExpressionNode {
  readonly kind = 'StringExpression';
  readonly parts: IExpressionNode[];

  constructor(parts: IExpressionNode[]) {
    super();
    this.parts = parts;
  }

  accept(visitor: IVisitor) {
    for (const part of this.parts) {
      part.accept(visitor);
    }

    visitor.visitStringExpression(this);
  }
}
