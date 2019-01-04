interface Location {
  lineStart: number;
  lineEnd: number;
  colStart: number;
  colEnd: number;
}

interface Token {
  type: 'symbol' | 'operator' | 'number' | 'string' | 'boolean';
  value: any;
  location: Location;
}

interface SymbolToken extends Token {
  type: 'symbol';
  value: '=' | '{' | '}' | '(' | ')' | '.' | ',' | '[' | ']';
}

interface OperatorToken extends Token {
  type: 'operator';
  value: '@' | '<' | '>' | '==' | '>=' | '<=' | '!=';
}

interface NumberToken extends Token {
  type: 'number';
  value: number;
}

interface BooleanToken extends Token {
  type: 'boolean';
  value: boolean;
}
