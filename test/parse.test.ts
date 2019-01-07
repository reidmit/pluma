import { parseModule } from '../src/parse';
import { tokenize } from '../src/tokenize';

const toAst = (source: string) => parseModule(tokenize(source), source);

describe('parser', () => {
  test('expressions in parentheses', () => {
    expect(toAst('((true))')).toMatchSnapshot();
    expect(toAst('(wow)')).toMatchSnapshot();
  });

  test('inline comments', () => {
    expect(toAst('# will be ignored')).toMatchSnapshot();
    expect(toAst('wow # will be ignored')).toMatchSnapshot();
    expect(
      toAst(`
        hello
        # will be ignored
        world
    `)
    ).toMatchSnapshot();
    expect(
      toAst(`
        hello
        # will be ignored
        # will also be ignored
        world
    `)
    ).toMatchSnapshot();
  });

  test('boolean literals', () => {
    expect(toAst('true')).toMatchSnapshot();
    expect(toAst('false')).toMatchSnapshot();
  });

  test('number literals', () => {
    expect(toAst('47')).toMatchSnapshot();
    expect(toAst('47.123')).toMatchSnapshot();
    expect(toAst('47e3')).toMatchSnapshot();
    expect(toAst('0b01010')).toMatchSnapshot();
  });

  test('string literals', () => {
    expect(toAst('"hello, world!"')).toMatchSnapshot();
    expect(
      toAst(`"
    hello,
      world!"
    `)
    ).toMatchSnapshot();
    expect(
      toAst(`"""
    hello,
      world!"""
    `)
    ).toMatchSnapshot();
  });

  test('plain identifiers', () => {
    expect(toAst('wow')).toMatchSnapshot();
    expect(toAst('hello')).toMatchSnapshot();
  });

  test('blocks without parameters', () => {
    expect(toAst('{ wow }')).toMatchSnapshot();
    expect(toAst('{ true }')).toMatchSnapshot();
  });

  test('blocks with parameters', () => {
    expect(toAst('{ a, b => wow }')).toMatchSnapshot();
    expect(toAst('{ hello => wow }')).toMatchSnapshot();
  });

  test('assignment expressions', () => {
    expect(toAst('a = 47')).toMatchSnapshot();
    expect(toAst('a = b = 10')).toMatchSnapshot();
    expect(
      toAst(`a = """
      a
      long
      string
    """`)
    ).toMatchSnapshot();
    expect(
      toAst(`a = { wow =>
      aBlock
    }`)
    ).toMatchSnapshot();
  });
});
