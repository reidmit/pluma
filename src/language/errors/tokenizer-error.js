import { formatSourceBlock } from './error-helper';

class TokenizerError extends Error {
  constructor(baseMessage, source, lineNumber, column) {
    const message =
      `${baseMessage} at line ${lineNumber}, column ${column}:` +
      '\n\n' +
      formatSourceBlock({ source, lineNumber, columnStart: column });

    super(message);
    this.name = 'Lexer error';
    this.stack = null;
  }
}

export default TokenizerError;
