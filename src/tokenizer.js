import { tokenTypes, symbols, reservedWords } from './constants';

const patterns = {
  booleanLiteral: /^(true|false)$/,
  hexNumericLiteral: /^0x[\da-f]+$/i,
  octalNumericLiteral: /^0o[0-7]+$/i,
  binaryNumericLiteral: /^0b[01]+$/i,
  specialNumericLiteral: /^(NaN|Infinity)$/,
  digits: /^[0-9]+$/
};

const isWordSeparator = char =>
  char === ' ' ||
  char === '\n' ||
  char === '\t' ||
  char === '\r' ||
  !!symbols[char];

const tokenize = source => {
  const length = source.length;
  const tokens = [];
  let buffer = [];
  let line = 1;
  let column = -1;

  const pushToken = (type, text, value, columnAdvance = 0) => {
    tokens.push({
      type,
      value,
      lineStart: line,
      lineEnd: line,
      columnStart: column + columnAdvance - text.length,
      columnEnd: column + columnAdvance
    });

    buffer = [];
  };

  for (let i = 0; i < length; i++) {
    column++;
    const char = source[i];

    if (isWordSeparator(char)) {
      if (buffer.length) {
        const text = buffer.join('');

        if (patterns.booleanLiteral.test(text)) {
          pushToken(tokenTypes.BOOLEAN, text, text === 'true');
        } else if (patterns.hexNumericLiteral.test(text)) {
          pushToken(tokenTypes.NUMBER, text, parseInt(text.substring(2), 16));
        } else if (patterns.octalNumericLiteral.test(text)) {
          pushToken(tokenTypes.NUMBER, text, parseInt(text.substring(2), 8));
        } else if (patterns.binaryNumericLiteral.test(text)) {
          pushToken(tokenTypes.NUMBER, text, parseInt(text.substring(2), 2));
        } else if (patterns.specialNumericLiteral.test(text)) {
          pushToken(tokenTypes.NUMBER, text, +text);
        } else if (!isNaN(parseFloat(text))) {
          pushToken(tokenTypes.NUMBER, text, parseFloat(text));
        } else if (text === 'null') {
          pushToken(tokenTypes.NULL, text, null);
        } else if (text === 'undefined') {
          pushToken(tokenTypes.UNDEFINED, text, undefined);
        } else if (reservedWords[text]) {
          pushToken(tokenTypes.KEYWORD, text, text);
        }
      } else {
        console.log({ char, buffer });
        if (char === '.' && patterns.digits.test(buffer.join(''))) {
          // special case: if we find a '.', but the buffer contains
          // the first part of a decimal number, continue on to let it
          // be tokenized as a number
          console.log('special');
          buffer.push(char);
          continue;
        }

        const nextTwoChars = i < length - 1 ? char + source[i + 1] : '';
        if (symbols[nextTwoChars]) {
          pushToken(symbols[nextTwoChars], nextTwoChars, nextTwoChars, 2);
          column++;
          i++;
        } else if (symbols[char]) {
          pushToken(symbols[char], char, char, 1);
        }
      }

      if (char === '\n') {
        column = -1;
        line++;
      }
    } else {
      buffer.push(char);
    }
  }

  return tokens;
};

export { tokenize };
