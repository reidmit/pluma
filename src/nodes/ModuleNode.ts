import { BaseNode } from './BaseNode';
import { ISyntaxNode } from '../types';
import { IVisitor } from '../visitor';

export interface IModuleNode extends ISyntaxNode {
  kind: 'Module';
  body: ISyntaxNode[];
}

export class ModuleNode extends BaseNode implements IModuleNode {
  readonly kind = 'Module';
  readonly body: ISyntaxNode[];

  constructor(body: ISyntaxNode[]) {
    super();
    this.body = body;
  }

  accept(visitor: IVisitor) {
    for (const node of this.body) {
      node.accept(visitor);
    }

    visitor.visitModule(this);
  }
}
