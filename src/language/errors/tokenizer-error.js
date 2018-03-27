class TokenizerError extends Error {
  constructor(message, source, line, column) {
    super(TokenizerError.formatMessage(message, source, line, column));
    this.name = 'Lexer error';
    this.stack = null;
  }

  static formatMessage(message, source, lineNumber, column) {
    const sourceLines = source.split('\n');
    const nearbyLines = sourceLines
      .map((lineText, lineIndex) => {
        const isLineWithError = lineIndex + 1 === lineNumber;
        const linePrefix = `${isLineWithError ? ' > ' : '   '}${lineIndex +
          1} | `;
        return (
          `${linePrefix}${lineText}` +
          (isLineWithError
            ? '\n' + Array(column + linePrefix.length + 1).join(' ') + '^'
            : '')
        );
      })
      .filter(
        (_, lineIndex) =>
          lineIndex + 1 >= lineNumber - 2 && lineIndex + 1 <= lineNumber + 2
      );

    const maxBarIndex = nearbyLines.reduce((max, lineText) => {
      const barIndex = lineText.indexOf('|');
      return max > barIndex ? max : barIndex;
    }, 0);

    const paddedNearbyLines = nearbyLines.map(lineText => {
      const barIndex = lineText.indexOf('|');

      if (barIndex < maxBarIndex) {
        return ' '.repeat(maxBarIndex - barIndex) + lineText;
      }

      return lineText;
    });

    return (
      `Unrecognized character '${
        sourceLines[lineNumber - 1][column]
      }' (line ${lineNumber}, column ${column})` +
      '\n\n' +
      paddedNearbyLines.join('\n')
    );
  }
}

export default TokenizerError;
