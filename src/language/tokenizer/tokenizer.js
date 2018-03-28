import {
  tokenTypes,
  symbols,
  symbolRegexes,
  reservedWords,
  reservedWordRegexes
} from '../constants';

import { TokenizerError } from '../errors';

const patterns = {
  newline: /^[\n]+/,
  otherWhitespace: /^[ \t\r]+/,
  booleanLiteral: /^(True|False)/,
  decimalNumericLiteral: /^[\d]+\.?[\d]*/,
  hexNumericLiteral: /^0x[\da-f]+/i,
  octalNumericLiteral: /^0o[0-7]+/i,
  binaryNumericLiteral: /^0b[01]+/i,
  specialNumericLiteral: /^(NaN|Infinity)/,
  identifier: /^[a-z$_][0-9a-z$_]*/i,
  digits: /^[0-9]+/
};

function tokenize({ source }) {
  const length = source.length;
  const tokens = [];
  const interpolationStack = [];
  let line = 1;
  let column = 0;
  let i = 0;
  let inString = false;
  let stringStart = null;
  let inRegex = false;
  let regexStart = null;
  let match;
  let remaining;

  function fail(
    message,
    { lineNumber = line, columnStart = column, columnEnd = column, hint } = {}
  ) {
    throw new TokenizerError(
      message,
      source,
      lineNumber,
      columnStart,
      columnEnd,
      hint
    );
  }

  function pushToken(type, text, value) {
    tokens.push({
      type,
      value,
      lineStart: line,
      lineEnd: line,
      columnStart: column,
      columnEnd: column + text.length
    });
  }

  function advance(amount = 1) {
    i += amount;
    column += amount;
  }

  outer: while (i < length) {
    remaining = source.substring(i);

    if (inString) {
      if (remaining[0] === '\n') {
        i++;
        column = 0;
        line++;
        continue;
      }

      const endQuote = remaining[0] === '"';
      const startInterpolation = remaining[0] === '$' && remaining[1] === '{';
      if (endQuote || startInterpolation) {
        const { charIndex, lineStart, columnStart } = stringStart;
        stringStart = null;

        tokens.push({
          type: tokenTypes.STRING,
          value: source.substring(charIndex, i),
          lineStart,
          lineEnd: line,
          columnStart,
          columnEnd: endQuote ? column + 1 : column
        });

        inString = false;
        if (endQuote) {
          advance();
        } else {
          interpolationStack.push(true);
        }

        continue;
      } else {
        advance();
        continue;
      }
    }

    if (
      inRegex &&
      (remaining[0] !== '/' || (remaining[0] === '/' && source[i - 1] === '\\'))
    ) {
      advance();
      continue;
    }

    if (remaining[0] === '"') {
      inString = true;
      stringStart = { lineStart: line, columnStart: column, charIndex: i + 1 };
      advance();
      continue;
    } else if (interpolationStack.length && remaining[0] === '}') {
      inString = true;
      stringStart = {
        lineStart: line,
        columnStart: column + 1,
        charIndex: i + 1
      };
      interpolationStack.pop();
      pushToken(tokenTypes.SYMBOL, '}', '}');
      advance();
      continue;
    }

    if ((match = remaining.match(patterns.newline))) {
      i += match[0].length;
      column = 0;
      line += match[0].length;
      continue;
    }

    if ((match = remaining.match(patterns.otherWhitespace))) {
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.booleanLiteral))) {
      pushToken(tokenTypes.BOOLEAN, match[0], match[0] === 'True');
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.hexNumericLiteral))) {
      pushToken(
        tokenTypes.NUMBER,
        match[0],
        parseInt(match[0].substring(2), 16)
      );
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.octalNumericLiteral))) {
      pushToken(
        tokenTypes.NUMBER,
        match[0],
        parseInt(match[0].substring(2), 8)
      );
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.binaryNumericLiteral))) {
      pushToken(
        tokenTypes.NUMBER,
        match[0],
        parseInt(match[0].substring(2), 2)
      );
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.decimalNumericLiteral))) {
      pushToken(tokenTypes.NUMBER, match[0], parseFloat(match[0]));
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.specialNumericLiteral))) {
      pushToken(tokenTypes.NUMBER, match[0], +match[0]);
      advance(match[0].length);
      continue;
    }

    if (remaining[0] === '/') {
      if (!inRegex) {
        inRegex = true;
        regexStart = {
          lineStart: line,
          columnStart: column,
          charIndex: i + 1
        };
        advance();
      } else {
        advance();
        inRegex = false;
        const { lineStart, columnStart, charIndex } = regexStart;
        regexStart = null;

        const regexString = source.substring(charIndex, i - 1);

        let regexFlags;
        if ((match = source.substring(i).match(/^[gimuy]+/))) {
          regexFlags = match[0];
          advance(match[0].length);
        }

        let value;
        try {
          value = new RegExp(regexString, regexFlags);
        } catch (err) {
          let hint;
          if (err.message.indexOf('Unterminated group') > -1) {
            hint =
              'It looks like you may be missing a closing ")" for a group.';
          } else if (err.message.indexOf('Unmatched ")"')) {
            hint =
              'It looks like you have a closing ")" without an opening "(".';
          }

          fail('Invalid regular expression', {
            lineStart,
            columnStart,
            columnEnd: column,
            hint
          });
        }

        tokens.push({
          type: tokenTypes.REGEX,
          value,
          lineStart,
          lineEnd: line,
          columnStart,
          columnEnd: column
        });
      }
      continue;
    }

    if (remaining.lastIndexOf('null', 0) === 0) {
      pushToken(tokenTypes.NULL, 'null', null);
      advance(4);
      continue;
    }

    if (remaining.lastIndexOf('undefined', 0) === 0) {
      pushToken(tokenTypes.UNDEFINED, 'undefined', undefined);
      advance(9);
      continue;
    }

    for (let w = 0; w < reservedWords.length; w++) {
      if (reservedWordRegexes[w].test(remaining)) {
        pushToken(tokenTypes.KEYWORD, reservedWords[w], reservedWords[w]);
        advance(reservedWords[w].length);
        continue outer;
      }
    }

    for (let s = 0; s < symbols.length; s++) {
      if (symbolRegexes[s].test(remaining)) {
        pushToken(tokenTypes.SYMBOL, symbols[s], symbols[s]);
        advance(symbols[s].length);
        continue outer;
      }
    }

    if ((match = remaining.match(patterns.identifier))) {
      pushToken(tokenTypes.IDENTIFIER, match[0], match[0]);
      advance(match[0].length);
      continue;
    }

    fail(`Unrecognized character '${remaining[0]}'`);
  }

  return tokens;
}

export default tokenize;
