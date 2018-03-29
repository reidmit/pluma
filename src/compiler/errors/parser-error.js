import { formatSourceBlock } from './error-helper';

class ParserError extends Error {
  constructor(baseMessage, source, token) {
    const { lineStart, columnStart, columnEnd } = token;

    const message =
      `at line ${lineStart}, ` +
      `columns ${columnStart}-${columnEnd}` +
      ':\n\n' +
      baseMessage +
      '\n\n' +
      formatSourceBlock({
        source,
        lineNumber: lineStart,
        columnStart,
        columnEnd
      });

    super(message);
    this.name = 'Parser error';
    this.stack = null;
  }
}

export default ParserError;
