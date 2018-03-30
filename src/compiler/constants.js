export const nodeTypes = {
  ARRAY: 'array-expression',
  ASSIGNMENT: 'assignment-node',
  BOOLEAN: 'boolean-node',
  CALL: 'call-node',
  CONDITIONAL: 'conditional-node',
  FUNCTION: 'function-node',
  IDENTIFIER: 'identifier-node',
  INTERPOLATED_STRING: 'interpolated-string-node',
  MEMBER_EXPRESSION: 'member-expression-node',
  MODULE: 'module-node',
  NUMBER: 'number-node',
  OBJECT: 'object-node',
  OBJECT_PROPERTY: 'object-property-node',
  STRING: 'string-node',
  TUPLE: 'tuple-node'
};

export const tokenTypes = {
  AT_IDENTIFIER: 'at-identifier-token',
  BOOLEAN: 'boolean-token',
  DOT_IDENTIFIER: 'dot-identifier-token',
  IDENTIFIER: 'identifier-token',
  KEYWORD: 'keyword-token',
  LINE_COMMENT: 'line-comment-token',
  NUMBER: 'number-token',
  REGEX: 'regex-token',
  SETTER: 'setter-token',
  STRING: 'string-token',
  SYMBOL: 'symbol-token'
};

export const symbols = [
  '(',
  ')',
  '{',
  '}',
  '[',
  ']',
  '${',
  ':',
  '=>',
  '->',
  '=',
  '.',
  ',',
  '|>'
];

export const symbolRegexes = symbols.map(
  symbol =>
    new RegExp(
      '^' +
        symbol
          .split('')
          .map(char => '\\' + char)
          .join('')
    )
);

export const reservedWords = ['let', 'if', 'then', 'else'];

export const reservedWordRegexes = reservedWords.map(
  word => new RegExp('^' + word + '\\b')
);
