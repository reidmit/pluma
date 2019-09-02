import checkTypes from '../src/type-checker';
import tokenize from '../src/tokenizer';
import parse from '../src/parser';

const source = `
  47

  [x => 36, y => 37, z => 47]

  q = {a: 47, b: "nice", c: x => 400}

  q.a
`;

const tokens = tokenize({ source });
const ast = parse({ tokens, source });

xdescribe('type checker', () => {
  test('temp', () => {
    checkTypes({ ast });
  });
});
