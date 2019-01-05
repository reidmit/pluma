type TokenType =
  | 'arrow'
  | 'boolean'
  | 'colon'
  | 'comma'
  | 'comment'
  | 'dot'
  | 'double-arrow'
  | 'equals'
  | 'identifier'
  | 'interpolation-end'
  | 'interpolation-start'
  | 'l-brace'
  | 'l-bracket'
  | 'l-paren'
  | 'operator'
  | 'number'
  | 'r-brace'
  | 'r-bracket'
  | 'r-paren'
  | 'string';

interface SourceLocation {
  lineStart: number;
  lineEnd: number;
  colStart: number;
  colEnd: number;
}

interface Token {
  type: TokenType;
  value?: string;
  location: SourceLocation;
}
