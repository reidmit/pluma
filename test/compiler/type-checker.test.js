import checkTypes from '../../src/compiler/type-checker';
import tokenize from '../../src/compiler/tokenizer';
import parse from '../../src/compiler/parser';

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
