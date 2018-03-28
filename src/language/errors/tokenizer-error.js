import { formatSourceBlock } from './error-helper';

class TokenizerError extends Error {
  constructor(baseMessage, source, lineNumber, columnStart, columnEnd, hint) {
    const message =
      `${baseMessage} at line ${lineNumber}, ` +
      `column ${columnStart}:` +
      '\n\n' +
      formatSourceBlock({ source, lineNumber, columnStart, columnEnd }) +
      (hint ? '\n\n' + hint : '');

    super(message);
    this.name = 'Lexer error';
    this.stack = null;
  }
}

export default TokenizerError;
