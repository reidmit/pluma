import { formatSourceBlock } from './error-helper';

class TokenizerError extends Error {
  constructor(baseMessage, source, lineNumber, columnStart, columnEnd, hint) {
    const message =
      '\n\n' +
      baseMessage +
      formatSourceBlock({ source, lineNumber, columnStart, columnEnd }) +
      (hint ? '\n\n' + hint : '');

    super(message);
    this.name = `Syntax error at line ${lineNumber}, column ${columnStart}`;
    this.stack = null;
  }
}

export default TokenizerError;
