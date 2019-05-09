export class ParseError extends Error {
  readonly line: number;
  readonly col: number;

  constructor(line: number, col: number, message: string) {
    super(`Parse error at line ${line}, column ${col}:\n\n${message}`);
    this.line = line;
    this.col = col;
  }
}
