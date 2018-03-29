import checkTypes from './type-checker';
import tokenize from '../tokenizer';
import parse from '../parser';

const source = `
  47

  [x => 36, y => 37, z => 47]

  let q = {a: 47, b: "nice", c: x => 400}

  q.a
`;

const tokens = tokenize({ source });
const ast = parse({ tokens, source });

xdescribe('type checker', () => {
  test('temp', () => {
    checkTypes({ ast });
  });
});
