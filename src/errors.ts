import { Token } from './tokens';
import { Parser } from './parse';
import { ISourceLocation } from './types';
import { Tokenizer } from './tokenize';

interface IErrorLocation {
  fileName: string;
  source: string;
  lineStart: number;
  colStart: number;
  lineEnd: number;
  colEnd: number;
}

export class ParseError extends Error {
  static fromLineAndColumn(message: string, line: number, col: number, tokenizer: Tokenizer) {
    return new ParseError(message, {
      fileName: tokenizer.fileName,
      source: tokenizer.source,
      lineStart: line,
      colStart: col,
      lineEnd: line,
      colEnd: col + 1
    });
  }

  static fromToken(message: string, token: Token, parser: Parser) {
    return new ParseError(message, {
      fileName: parser.fileName,
      source: parser.source,
      lineStart: token.lineStart,
      colStart: token.colStart,
      lineEnd: token.lineEnd,
      colEnd: token.colEnd
    });
  }

  static fromSourceLocation(message: string, sourceLocation: ISourceLocation, parser: Parser) {
    return new ParseError(message, {
      fileName: parser.fileName,
      source: parser.source,
      lineStart: sourceLocation.lineStart,
      colStart: sourceLocation.colStart,
      lineEnd: sourceLocation.lineEnd,
      colEnd: sourceLocation.colEnd
    });
  }

  private constructor(message: string, location: IErrorLocation) {
    const { fileName, lineStart, colStart } = location;

    super(
      `Parse error${
        fileName ? ` in ${fileName}` : ''
      } at line ${lineStart}, column ${colStart}:\n\n${message}`
    );

    this.stack = '';
  }
}
