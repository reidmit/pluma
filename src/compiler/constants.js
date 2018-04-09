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
