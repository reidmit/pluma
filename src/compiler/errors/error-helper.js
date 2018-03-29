import chalk from 'chalk';
import stringLength from 'string-length';
import { tokenTypes } from '../constants';

/**
 * Given a character and a number, returns a string with that character repeated
 * that number of times.
 *
 * ex: repeatChar('x', 4) === 'xxxx'
 */
function repeatChar(char, times) {
  let repeated = '';
  while (times-- > 0) repeated += char;
  return repeated;
}

/**
 * Given some source code and information about where an error is located
 * within the code, returns a string that highlights the error within the
 * source. The resulting string looks something like this (note the arrows):
 *
 *    2 | let aa = 1
 *  > 3 | let someError = 3
 *            ^^^^^^^^^
 *    4 | let cc = 5
 */
function formatSourceBlock({
  source,
  lineNumber,
  columnStart,
  columnEnd = columnStart,
  surroundingLines = 2,
  useColor = true
}) {
  const sourceLines = source.split('\n');
  const lineTexts = [];
  const prefixes = [];
  const formattedLines = [];
  let maxPrefixLength = 0;
  let carets = repeatChar('^', Math.max(1, columnEnd - columnStart));
  let rightArrow = '>';

  if (useColor) {
    carets = chalk.red(carets);
    rightArrow = chalk.red(rightArrow);
  }

  for (
    let i = Math.max(0, lineNumber - 1 - surroundingLines);
    i <= Math.min(sourceLines.length, lineNumber - 1 + surroundingLines);
    i++
  ) {
    if (i >= 0 && i < sourceLines.length) {
      const lineText = sourceLines[i];
      const isLineWithError = i + 1 === lineNumber;
      const prefix =
        ` ${isLineWithError ? rightArrow : ''} ` +
        (useColor ? chalk.gray(`${i + 1} | `) : `${i + 1} | `);
      const prefixLength = stringLength(prefix);
      maxPrefixLength = Math.max(maxPrefixLength, prefixLength);
      prefixes.push(prefix);
      lineTexts.push(lineText);

      if (isLineWithError) {
        prefixes.push(repeatChar(' ', prefixLength + columnStart));
        lineTexts.push(carets);
      }
    }
  }

  for (let i = 0; i < lineTexts.length; i++) {
    let prefix = prefixes[i];
    const prefixLength = stringLength(prefix);
    if (prefixLength < maxPrefixLength) {
      prefix = repeatChar(' ', maxPrefixLength - prefixLength) + prefix;
    }
    formattedLines.push(prefix + lineTexts[i]);
  }

  return formattedLines.join('\n');
}

function tokenToString(token) {
  switch (token.type) {
    case tokenTypes.IDENTIFIER:
      return `identifier "${token.value}"`;
    case tokenTypes.KEYWORD:
      return `keyword "${token.value}"`;
    case tokenTypes.SYMBOL:
      return `symbol "${token.value}"`;
    case tokenTypes.NUMBER:
      return `number ${token.value}`;
    case tokenTypes.STRING:
      return `string "${token.value}"`;
    default:
      return 'token';
  }
}

export { formatSourceBlock, tokenToString };
