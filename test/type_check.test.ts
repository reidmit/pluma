import { typeCheck } from '../src/type_check';
import { parse } from '../src/parse';

describe('typeCheck', () => {
  test('number literal', () => {
    expect(typeCheck(parse('47'))).toMatchSnapshot();
  });
});
