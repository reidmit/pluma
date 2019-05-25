import { IExpressionNode } from '../types';
import { IIdentifierNode } from './IdentifierNode';
import { BaseExpressionNode } from './BaseExpressionNode';
import { IVisitor } from '../visitor';

export interface IBlockNode extends IExpressionNode {
  kind: 'Block';
  params: IIdentifierNode[];
  body: IExpressionNode[];
}

export class BlockNode extends BaseExpressionNode implements IBlockNode {
  readonly kind = 'Block';
  readonly params: IIdentifierNode[];
  readonly body: IExpressionNode[];

  constructor(params: IIdentifierNode[], body: IExpressionNode[]) {
    super();
    this.params = params;
    this.body = body;
  }

  accept(visitor: IVisitor) {
    for (const param of this.params) {
      param.accept(visitor);
    }

    for (const node of this.body) {
      node.accept(visitor);
    }

    visitor.visitBlock(this);
  }
}
