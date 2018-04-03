import { formatSourceBlock } from './error-helper';

class ParserError extends Error {
  constructor(baseMessage, source, token) {
    const { lineStart, columnStart, columnEnd } = token;

    const message =
      '\n\n' +
      baseMessage +
      '\n\n' +
      formatSourceBlock({
        source,
        lineNumber: lineStart,
        columnStart,
        columnEnd
      });

    super(message);
    this.name = `Syntax error at line ${lineStart}, columns ${columnStart}-${columnEnd}`;
    this.stack = null;
  }
}

export default ParserError;
