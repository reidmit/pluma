import { IIdentifierNode } from './IdentifierNode';
import { IExpressionNode } from '../types';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface ICallNode extends IExpressionNode {
  kind: 'Call';
  callee: IIdentifierNode;
  args: IExpressionNode[];
}

export class CallNode extends BaseExpressionNode implements ICallNode {
  readonly kind = 'Call';
  readonly callee: IIdentifierNode;
  readonly args: IExpressionNode[];

  constructor(callee: IIdentifierNode, args: IExpressionNode[]) {
    super();
    this.callee = callee;
    this.args = args;
  }

  accept(visitor: IVisitor) {
    this.callee.accept(visitor);

    for (const arg of this.args) {
      arg.accept(visitor);
    }

    visitor.visitCall(this);
  }
}
