import { typeCheck } from '../src/type_check';
import { parse } from '../src';

describe('typeCheck', () => {
  test('single literal expressions', () => {
    expect(typeCheck(parse('47'))).toMatchSnapshot();
    expect(typeCheck(parse('"wow"'))).toMatchSnapshot();
    expect(typeCheck(parse('true'))).toMatchSnapshot();
    expect(typeCheck(parse('false'))).toMatchSnapshot();
  });

  test('assignment expressions', () => {
    expect(typeCheck(parse('num = 47'))).toMatchSnapshot();
    expect(typeCheck(parse('str = "wow"'))).toMatchSnapshot();
  });

  test('array expressions', () => {
    expect(typeCheck(parse('[1, 2, 3]'))).toMatchSnapshot();
  });
});
