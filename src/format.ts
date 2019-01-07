import { ParseError } from './errors';
import { Visitor } from './visit';

class Formatter extends Visitor {
  format() {}
}

export function format(ast: AstNode, source: string): string {
  return new Formatter(ast, source).format();
}
