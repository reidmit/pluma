export class ParseError extends Error {
  constructor(line: number, col: number, message: string) {
    super(`Parse error at line ${line}, column ${col}:\n\n${message}`);

    this.stack = null;
  }
}
