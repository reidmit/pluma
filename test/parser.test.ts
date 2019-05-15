import { parseExpression, parseModule } from '../src/parse';

describe('parseExpression', () => {
  describe('simple expressions', () => {
    test('number literal', () => {
      expect(parseExpression('47')).toMatchSnapshot();
    });

    test('number literal with decimal', () => {
      expect(parseExpression('47.123')).toMatchSnapshot();
    });

    test('binary number literal', () => {
      expect(parseExpression('0b101011')).toMatchSnapshot();
    });

    test('hex number literal', () => {
      expect(parseExpression('0xfacade')).toMatchSnapshot();
    });

    test('octal number literal', () => {
      expect(parseExpression('0o1230')).toMatchSnapshot();
    });

    test('boolean literals', () => {
      expect(parseExpression('true')).toMatchSnapshot();
      expect(parseExpression('false')).toMatchSnapshot();
    });

    test('identifiers', () => {
      expect(parseExpression('wow')).toMatchSnapshot();
    });
  });

  describe('string expressions', () => {
    test('no interpolations', () => {
      expect(parseExpression('"hello!"')).toMatchSnapshot();
    });

    test('no interpolations, multi-line', () => {
      expect(parseExpression('"hello\n\nworld!"')).toMatchSnapshot();
    });

    test('single interpolation', () => {
      expect(parseExpression('"hello $(name)!"')).toMatchSnapshot();
    });

    test('multiple interpolations', () => {
      expect(parseExpression('"hello $(firstName) $(lastName)!"')).toMatchSnapshot();
    });

    test('nested interpolations', () => {
      expect(parseExpression('"this is $("weird but $("valid")")"')).toMatchSnapshot();
    });
  });

  describe('block expressions', () => {
    test('empty block', () => {
      expect(parseExpression('{}')).toMatchSnapshot();
    });

    test('block with single expression body', () => {
      expect(parseExpression('{ wow }')).toMatchSnapshot();
    });

    test('block with one param', () => {
      expect(parseExpression('{ p1 => "yep" }')).toMatchSnapshot();
    });

    test('block with two params', () => {
      expect(parseExpression('{ p1, p2 => "yep" }')).toMatchSnapshot();
    });

    test('multi-line block with no params and multple expressions in body', () => {
      expect(parseExpression('{\n  wow\n  "yep"\n  47\n}')).toMatchSnapshot();
    });

    test('multi-line block with params and multple expressions in body', () => {
      expect(parseExpression('{ p1, p2 => \n  wow\n  "yep"\n  47\n}')).toMatchSnapshot();
    });
  });

  describe('assignment expressions', () => {
    test('constant assignment', () => {
      expect(parseExpression('num = 47')).toMatchSnapshot();
    });

    test('variable assignment', () => {
      expect(parseExpression('num := 47')).toMatchSnapshot();
    });
  });

  describe('call expressions', () => {
    test('call without arguments', () => {
      expect(parseExpression('myFunction()')).toMatchSnapshot();
    });

    test('call with one argument', () => {
      expect(parseExpression('myFunction(47)')).toMatchSnapshot();
    });

    test('call with multiple arguments', () => {
      expect(parseExpression('myFunction("wow", great, 47, true)')).toMatchSnapshot();
    });
  });

  describe('array expressions', () => {
    test('empty array', () => {
      expect(parseExpression('[]')).toMatchSnapshot();
    });

    test('array with one element', () => {
      expect(parseExpression('[1]')).toMatchSnapshot();
    });

    test('array with multiple elements', () => {
      expect(parseExpression('[1, 2, 3]')).toMatchSnapshot();
    });

    test('array with trailing comma', () => {
      expect(parseExpression('[1, 2, 3, ]')).toMatchSnapshot();
    });

    test('multi-line array with multiple elements', () => {
      expect(parseExpression('[\n  1,\n  2,\n  3\n]')).toMatchSnapshot();
    });
  });
});

describe('parseModule', () => {
  describe('comments', () => {
    test('only one comment', () => {
      expect(parseModule('# the only comment')).toMatchSnapshot();
    });

    test('multiple comments, nothing else', () => {
      expect(parseModule('# first line\n# second line')).toMatchSnapshot();
    });

    test('comment on line before expression', () => {
      expect(parseModule('# a good number\nnum = 47')).toMatchSnapshot();
    });

    test('two comments on lines before expression', () => {
      expect(parseModule('# line one\n# line two\nnum = 47')).toMatchSnapshot();
    });

    test('two comments with blank line in between before expression', () => {
      expect(parseModule('# not associated\n\n# line one\nnum = 47')).toMatchSnapshot();
    });
  });
});
