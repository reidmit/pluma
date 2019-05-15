import { parse } from '../src/parse';

describe('parse', () => {
  describe('simple expressions', () => {
    test('number literal', () => {
      expect(parse('47')).toMatchSnapshot();
    });

    test('number literal with decimal', () => {
      expect(parse('47.123')).toMatchSnapshot();
    });

    test('binary number literal', () => {
      expect(parse('0b101011')).toMatchSnapshot();
    });

    test('hex number literal', () => {
      expect(parse('0xfacade')).toMatchSnapshot();
    });

    test('octal number literal', () => {
      expect(parse('0o1230')).toMatchSnapshot();
    });

    test('boolean literals', () => {
      expect(parse('true')).toMatchSnapshot();
      expect(parse('false')).toMatchSnapshot();
    });

    test('identifiers', () => {
      expect(parse('wow')).toMatchSnapshot();
    });
  });

  describe('string expressions', () => {
    test('no interpolations', () => {
      expect(parse('"hello!"')).toMatchSnapshot();
    });

    test('no interpolations, multi-line', () => {
      expect(parse('"hello\n\nworld!"')).toMatchSnapshot();
    });

    test('single interpolation', () => {
      expect(parse('"hello $(name)!"')).toMatchSnapshot();
    });

    test('multiple interpolations', () => {
      expect(parse('"hello $(firstName) $(lastName)!"')).toMatchSnapshot();
    });

    test('nested interpolations', () => {
      expect(parse('"this is $("weird but $("valid")")"')).toMatchSnapshot();
    });
  });

  describe('block expressions', () => {
    test('empty block', () => {
      expect(parse('{}')).toMatchSnapshot();
    });

    test('block with single expression body', () => {
      expect(parse('{ wow }')).toMatchSnapshot();
    });

    test('block with one param', () => {
      expect(parse('{ p1 => "yep" }')).toMatchSnapshot();
    });

    test('block with two params', () => {
      expect(parse('{ p1, p2 => "yep" }')).toMatchSnapshot();
    });

    test('multi-line block with no params and multple expressions in body', () => {
      expect(parse('{\n  wow\n  "yep"\n  47\n}')).toMatchSnapshot();
    });

    test('multi-line block with params and multple expressions in body', () => {
      expect(parse('{ p1, p2 => \n  wow\n  "yep"\n  47\n}')).toMatchSnapshot();
    });
  });

  describe('assignment expressions', () => {
    test('constant assignment', () => {
      expect(parse('num = 47')).toMatchSnapshot();
    });

    test('variable assignment', () => {
      expect(parse('num := 47')).toMatchSnapshot();
    });
  });

  describe('call expressions', () => {
    test('call without arguments', () => {
      expect(parse('myFunction()')).toMatchSnapshot();
    });

    test('call with one argument', () => {
      expect(parse('myFunction(47)')).toMatchSnapshot();
    });

    test('call with multiple arguments', () => {
      expect(parse('myFunction("wow", great, 47, true)')).toMatchSnapshot();
    });
  });

  describe('array expressions', () => {
    test('empty array', () => {
      expect(parse('[]')).toMatchSnapshot();
    });

    test('array with one element', () => {
      expect(parse('[1]')).toMatchSnapshot();
    });

    test('array with multiple elements', () => {
      expect(parse('[1, 2, 3]')).toMatchSnapshot();
    });

    test('array with trailing comma', () => {
      expect(parse('[1, 2, 3, ]')).toMatchSnapshot();
    });

    test('multi-line array with multiple elements', () => {
      expect(parse('[\n  1,\n  2,\n  3\n]')).toMatchSnapshot();
    });
  });

  describe('comments', () => {
    test('only one comment', () => {
      expect(parse('# the only comment')).toMatchSnapshot();
    });

    test('multiple comments, nothing else', () => {
      expect(parse('# first line\n# second line')).toMatchSnapshot();
    });

    test('comment on line before expression', () => {
      expect(parse('# a good number\nnum = 47')).toMatchSnapshot();
    });

    test('two comments on lines before expression', () => {
      expect(parse('# line one\n# line two\nnum = 47')).toMatchSnapshot();
    });

    test('two comments with blank line in between before expression', () => {
      expect(parse('# not associated\n\n# line one\nnum = 47')).toMatchSnapshot();
    });
  });
});
