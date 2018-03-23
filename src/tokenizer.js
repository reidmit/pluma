import {
  tokenTypes,
  symbols,
  symbolRegexes,
  reservedWords,
  reservedWordRegexes
} from './constants';

const patterns = {
  newline: /^[\n]+/,
  otherWhitespace: /^[ \t\r]+/,
  booleanLiteral: /^(true|false)/,
  decimalNumericLiteral: /^[\d]+\.?[\d]*/,
  hexNumericLiteral: /^0x[\da-f]+/i,
  octalNumericLiteral: /^0o[0-7]+/i,
  binaryNumericLiteral: /^0b[01]+/i,
  specialNumericLiteral: /^(NaN|Infinity)/,
  identifier: /^[a-z$_][0-9a-z$_]*/i,
  digits: /^[0-9]+/
};

const tokenize = ({ source }) => {
  const length = source.length;
  const tokens = [];
  const interpolationStack = [];
  let line = 1;
  let column = 0;
  let i = 0;
  let inString = false;
  let stringStart = null;
  let match;
  let remaining;

  const pushToken = (type, text, value, columnAdvance = 0) => {
    tokens.push({
      type,
      value,
      lineStart: line,
      lineEnd: line,
      columnStart: column + columnAdvance,
      columnEnd: column + columnAdvance + text.length
    });
  };

  const advance = amount => {
    i += amount;
    column += amount;
  };

  outer: while (i < length) {
    remaining = source.substring(i);

    if (inString) {
      if (remaining[0] === '\n') {
        i++;
        column = 0;
        line++;
        continue;
      }

      const endQuote = remaining[0] === '\'';
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
          advance(1);
        } else {
          interpolationStack.push(true);
        }

        continue;
      } else {
        advance(1);
        continue;
      }
    }

    if (remaining[0] === '\'') {
      inString = true;
      stringStart = { lineStart: line, columnStart: column, charIndex: i + 1 };
    } else if (interpolationStack.length && remaining[0] === '}') {
      inString = true;
      stringStart = {
        lineStart: line,
        columnStart: column + 1,
        charIndex: i + 1
      };
    }

    if ((match = remaining.match(patterns.newline))) {
      i += match[0].length;
      column = 0;
      line++;
      continue;
    }

    if ((match = remaining.match(patterns.otherWhitespace))) {
      advance(match[0].length);
      continue;
    }

    if ((match = remaining.match(patterns.booleanLiteral))) {
      pushToken(tokenTypes.BOOLEAN, match[0], match[0] === 'true');
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

    advance(1);
  }

  return tokens;
};

export { tokenize };
