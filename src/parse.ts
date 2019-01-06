class Parser {
  tokens: Token[];
  source: string;
  index: number;

  constructor(tokens: Token[], source: string) {
    this.tokens = tokens;
    this.source = source;
    this.index = 0;
  }

  parse(): File {}
}

export function parse(tokens: Token[], source: string): File {
  return new Parser(tokens, source).parse();
}
