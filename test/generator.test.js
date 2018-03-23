import { generate } from '../src/generator';

describe('generate', () => {
  test('temp', () => {
    const source = `
let fn = a => b => c => 'hello, world!'

fn 1 2 3
`;

    generate({ source });
  });
});
