import { BaseNode } from './BaseNode';
import { IExpressionNode } from '../types';

export abstract class BaseExpressionNode extends BaseNode implements IExpressionNode {
  type: string | null = null;
}
