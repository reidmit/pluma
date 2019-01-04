const symbolNames = {
  '=': 'equals',
  '(': 'lparen',
  ')': 'rparen',
  '[': 'lsquare',
  ']': 'rsquare',
  '{': 'lcurly',
  '}': 'rcurly',
  '.': 'dot',
  ',': 'comma',
  ':': 'colon'
};

const tokenTypes = {
  STRING: 0,
  NUMBER: 1
};

export function tokenize(input: string): Token[] {
  const length = input.length;
  const tokens: Token[] = [];

  let line = 0;
  let index = 0;

  while (index < length) {
    index++;
  }

  return tokens;
}
