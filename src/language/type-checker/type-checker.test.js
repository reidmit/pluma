import checkTypes from './type-checker';
import tokenize from '../tokenizer';
import parse from '../parser';

const source = `
  47

  "reid"

  let lol = 23

  lol

  let fn = x => 28

  let toStr = s => fn s

  [1, 2, 3]
`;

const tokens = tokenize({ source });
const ast = parse({ tokens, source });

describe('type checker', () => {
  test('temp', () => {
    checkTypes({ ast });
  });
});
