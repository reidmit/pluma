export const tokenTypes = {
  BOOLEAN: 'boolean-token',
  IDENTIFIER: 'identifier-token',
  DOT_IDENTIFIER: 'dot-identifier-token',
  AT_IDENTIFIER: 'at-identifier-token',
  NULL: 'null-token',
  NUMBER: 'number-token',
  REGEX: 'regex-token',
  SETTER: 'setter-token',
  STRING: 'string-token',
  SYMBOL: 'symbol-token',
  UNDEFINED: 'undefined-token'
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

export const reservedWords = ['else', 'if', 'let'];

export const reservedWordRegexes = reservedWords.map(
  word => new RegExp('^' + word + '\\b')
);
