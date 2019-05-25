import { BaseNode } from './BaseNode';
import { ISyntaxNode } from '../types';
import { IVisitor } from '../visitor';
import { IMemberExpressionNode } from './MemberExpressionNode';
import { IIdentifierNode } from './IdentifierNode';

export interface IModuleSpecifierNode extends ISyntaxNode {
  kind: 'ModuleSpecifier';
  name: IIdentifierNode | IMemberExpressionNode;
}

export class ModuleSpecifierNode extends BaseNode implements IModuleSpecifierNode {
  readonly kind = 'ModuleSpecifier';
  readonly name: IIdentifierNode | IMemberExpressionNode;

  constructor(name: IIdentifierNode | IMemberExpressionNode) {
    super();
    this.name = name;
  }

  accept(visitor: IVisitor) {
    visitor.visitModuleSpecifier(this);
  }
}
