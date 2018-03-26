import checkTypes from '../../src/language/type-checker';
import tokenize from '../../src/language/tokenizer';
import parse from '../../src/language/parser';

const source = `
  47

  "reid"

  let lol = 23

  lol

  let fn = x => 28

  let toStr = s => fn s


`;

const tokens = tokenize({ source });
const ast = parse({ tokens, source });

describe('type checker', () => {
  test('temp', () => {
    checkTypes({ ast });
  });
});
