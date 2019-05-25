import { BaseNode } from './BaseNode';
import { ISyntaxNode } from '../types';
import { IVisitor } from '../visitor';
import { IModuleSpecifierNode } from './ModuleSpecifierNode';

export interface IModuleNode extends ISyntaxNode {
  kind: 'Module';
  specifier: IModuleSpecifierNode | null;
  body: ISyntaxNode[];
}

export class ModuleNode extends BaseNode implements IModuleNode {
  readonly kind = 'Module';
  readonly specifier: IModuleSpecifierNode | null;
  readonly body: ISyntaxNode[];

  constructor(specifier: IModuleSpecifierNode | null, body: ISyntaxNode[]) {
    super();
    this.specifier = specifier;
    this.body = body;
  }

  accept(visitor: IVisitor) {
    if (this.specifier) this.specifier.accept(visitor);

    for (const node of this.body) {
      node.accept(visitor);
    }

    visitor.visitModule(this);
  }
}
