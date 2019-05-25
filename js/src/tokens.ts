export type TokenKind =
  | 'Boolean'
  | 'Char'
  | 'Comment'
  | 'Identifier'
  | 'InterpolationEnd'
  | 'InterpolationStart'
  | 'Operator'
  | 'Number'
  | 'String'
  | '->'
  | '=>'
  | '::'
  | ':'
  | ':='
  | ','
  | '.'
  | '='
  | '{'
  | '}'
  | '['
  | ']'
  | '('
  | ')';

export interface Token {
  kind: TokenKind;
  value: string;
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
}
