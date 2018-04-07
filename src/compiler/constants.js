export const nodeTypes = {
  ARRAY: 'array-expression',
  ASSIGNMENT: 'assignment-node',
  BOOLEAN: 'boolean-node',
  CALL: 'call-node',
  CONDITIONAL: 'conditional-node',
  FUNCTION: 'function-node',
  FUNCTION_TYPE: 'function-type-node',
  IDENTIFIER: 'identifier-node',
  INTERPOLATED_STRING: 'interpolated-string-node',
  MEMBER_EXPRESSION: 'member-expression-node',
  MODULE: 'module-node',
  NUMBER: 'number-node',
  OBJECT: 'object-node',
  OBJECT_PROPERTY: 'object-property-node',
  RECORD_TYPE: 'record-type-node',
  RECORD_TYPE_ENTRY: 'record-type-entry-node',
  STRING: 'string-node',
  TUPLE: 'tuple-node',
  TYPE_ALIAS_DECLARATION: 'type-alias-declaration-node',
  TYPE_CONSTRUCTOR: 'type-constructor-node',
  TYPE_DECLARATION: 'type-declaration-node',
  TYPE_TAG: 'type-tag-node',
  TYPE_TUPLE: 'type-tuple-node',
  TYPE_VARIABLE: 'type-variable-node'
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
  '::',
  ':',
  '=>',
  '->',
  '=',
  '.',
  ',',
  '|>',
  '|'
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

export const reservedWords = ['let', 'if', 'then', 'else', 'type', 'alias'];

export const reservedWordRegexes = reservedWords.map(
  word => new RegExp('^' + word + '\\b')
);
