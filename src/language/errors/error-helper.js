import chalk from 'chalk';
import stringLength from 'string-length';

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
    let i = lineNumber - 1 - surroundingLines;
    i <= lineNumber - 1 + surroundingLines;
    i++
  ) {
    if (i > 0 && i < sourceLines.length) {
      const lineText = sourceLines[i];
      const isLineWithError = i + 1 === lineNumber;
      const prefix = ` ${isLineWithError ? rightArrow : ''} ${i + 1} | `;
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
    if (prefix.length < maxPrefixLength) {
      prefix = repeatChar(' ', maxPrefixLength - prefix.length) + prefix;
    }
    formattedLines.push(prefix + lineTexts[i]);
  }

  return formattedLines.join('\n');
}

export { formatSourceBlock };
