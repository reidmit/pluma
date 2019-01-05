import { tokenize } from '../src/tokenize';

describe('tokenizer', () => {
  let tokens;

  test('line numbers', () => {
    tokens = tokenize('a bb ccc');

    expect(tokens).toHaveLength(3);

    expect(tokens[0].location.lineStart).toBe(1);
    expect(tokens[0].location.lineEnd).toBe(1);

    expect(tokens[1].location.lineStart).toBe(1);
    expect(tokens[1].location.lineEnd).toBe(1);

    expect(tokens[2].location.lineStart).toBe(1);
    expect(tokens[2].location.lineEnd).toBe(1);

    tokens = tokenize(`
  a

         bbbb

        cc
      `);

    expect(tokens).toHaveLength(3);

    expect(tokens[0].location.lineStart).toBe(2);
    expect(tokens[0].location.lineEnd).toBe(2);

    expect(tokens[1].location.lineStart).toBe(4);
    expect(tokens[1].location.lineEnd).toBe(4);

    expect(tokens[2].location.lineStart).toBe(6);
    expect(tokens[2].location.lineEnd).toBe(6);

    tokens = tokenize(`a "hello

      world" bb

         ccc
      `);

    expect(tokens).toHaveLength(4);

    expect(tokens[0].location.lineStart).toBe(1);
    expect(tokens[0].location.lineEnd).toBe(1);

    expect(tokens[1].location.lineStart).toBe(1);
    expect(tokens[1].location.lineEnd).toBe(3);

    expect(tokens[2].location.lineStart).toBe(3);
    expect(tokens[2].location.lineEnd).toBe(3);

    expect(tokens[3].location.lineStart).toBe(5);
    expect(tokens[3].location.lineEnd).toBe(5);

    tokens = tokenize(`"hello $(
        a

        bb
        ccc
      ) world"
      `);

    expect(tokens).toHaveLength(7);

    expect(tokens[0].location.lineStart).toBe(1);
    expect(tokens[0].location.lineEnd).toBe(1);

    expect(tokens[1].location.lineStart).toBe(1);
    expect(tokens[1].location.lineEnd).toBe(1);

    expect(tokens[2].location.lineStart).toBe(2);
    expect(tokens[2].location.lineEnd).toBe(2);

    expect(tokens[3].location.lineStart).toBe(4);
    expect(tokens[3].location.lineEnd).toBe(4);

    expect(tokens[4].location.lineStart).toBe(5);
    expect(tokens[4].location.lineEnd).toBe(5);

    expect(tokens[5].location.lineStart).toBe(6);
    expect(tokens[5].location.lineEnd).toBe(6);

    expect(tokens[6].location.lineStart).toBe(6);
    expect(tokens[6].location.lineEnd).toBe(6);

    tokens = tokenize(`"hello
      $(
        a
        bb)
    world"
    `);

    expect(tokens).toHaveLength(6);

    expect(tokens[0].location.lineStart).toBe(1);
    expect(tokens[0].location.lineEnd).toBe(2);

    expect(tokens[1].location.lineStart).toBe(2);
    expect(tokens[1].location.lineEnd).toBe(2);

    expect(tokens[2].location.lineStart).toBe(3);
    expect(tokens[2].location.lineEnd).toBe(3);

    expect(tokens[3].location.lineStart).toBe(4);
    expect(tokens[3].location.lineEnd).toBe(4);

    expect(tokens[4].location.lineStart).toBe(4);
    expect(tokens[4].location.lineEnd).toBe(4);

    expect(tokens[5].location.lineStart).toBe(4);
    expect(tokens[5].location.lineEnd).toBe(5);
  });

  test('identifiers', () => {
    expect(tokenize('hello')).toMatchSnapshot();
    expect(tokenize('hello2')).toMatchSnapshot();
    expect(tokenize('hello world')).toMatchSnapshot();
    expect(tokenize('h_llo wor_d')).toMatchSnapshot();
    expect(tokenize('_ _ wow')).toMatchSnapshot();
    expect(
      tokenize(`
      a

      b

      c  d
      `)
    ).toMatchSnapshot();
  });

  test('comments', () => {
    expect(tokenize('# a comment!')).toMatchSnapshot();
    expect(tokenize('#   another comment!')).toMatchSnapshot();
    expect(
      tokenize(`
      a # wow!
    #hello
      b
      `)
    ).toMatchSnapshot();
  });

  test('booleans', () => {
    expect(tokenize('true')).toMatchSnapshot();
    expect(tokenize('false')).toMatchSnapshot();
    expect(tokenize('hello true world false')).toMatchSnapshot();
  });

  test('numbers', () => {
    expect(tokenize('47')).toMatchSnapshot();
    expect(tokenize('47.01')).toMatchSnapshot();
    expect(tokenize('hello 1.22 world 2')).toMatchSnapshot();
    expect(tokenize('47e100')).toMatchSnapshot();
    expect(tokenize('0x10')).toMatchSnapshot();
    expect(tokenize('0b1010')).toMatchSnapshot();
    expect(tokenize('0o122345')).toMatchSnapshot();
  });

  test('symbols', () => {
    expect(tokenize('{ wow }')).toMatchSnapshot();
    expect(tokenize('( wow )')).toMatchSnapshot();
    expect(tokenize('[ wow ]')).toMatchSnapshot();
    expect(tokenize('wow . com')).toMatchSnapshot();
    expect(tokenize('a, b, c')).toMatchSnapshot();
    expect(tokenize('a => b')).toMatchSnapshot();
    expect(tokenize('a -> b')).toMatchSnapshot();
    expect(tokenize('a = b')).toMatchSnapshot();
    expect(tokenize('a : b')).toMatchSnapshot();
  });

  test('operators', () => {
    expect(tokenize('a @ b')).toMatchSnapshot();
    expect(tokenize('a >= b')).toMatchSnapshot();
    expect(tokenize('a == b')).toMatchSnapshot();
    expect(tokenize('a != b')).toMatchSnapshot();
    expect(tokenize('a < b')).toMatchSnapshot();
    expect(tokenize('a > b')).toMatchSnapshot();
    expect(tokenize('a >=!@ b')).toMatchSnapshot();
    expect(tokenize('a @@ b')).toMatchSnapshot();
    expect(tokenize('a + b')).toMatchSnapshot();
    expect(tokenize('a * b')).toMatchSnapshot();
    expect(tokenize('a - b')).toMatchSnapshot();
  });

  test('basic strings', () => {
    expect(tokenize('"hello world"')).toMatchSnapshot();
    expect(tokenize('"hello" "world"')).toMatchSnapshot();
    expect(tokenize('"hello\\"world"')).toMatchSnapshot();
    expect(tokenize('hello "world" hi')).toMatchSnapshot();
    expect(tokenize('empty "" nice')).toMatchSnapshot();
    expect(
      tokenize(`"hello

    world"`)
    ).toMatchSnapshot();
  });

  test('triple-quoted strings', () => {
    expect(tokenize('"""hello world"""')).toMatchSnapshot();
    expect(tokenize('"""hello "world" ok"""')).toMatchSnapshot();
    expect(tokenize('"""hello "world ok"""')).toMatchSnapshot();
    expect(
      tokenize(`"""hello

      "world ok"""

    `)
    ).toMatchSnapshot();
  });

  test('interpolated strings', () => {
    expect(tokenize('"hello $(name)"')).toMatchSnapshot();
    expect(tokenize('"hello \\$(name)"')).toMatchSnapshot();
    expect(tokenize('"hello $(hi there)"')).toMatchSnapshot();
    expect(
      tokenize(`"hello $(hi

      there)"`)
    ).toMatchSnapshot();
    expect(tokenize('"hello $(hi (there))"')).toMatchSnapshot();
    expect(tokenize('"hello $("another string")"')).toMatchSnapshot();
    expect(tokenize('"hello $(wow "another string")"')).toMatchSnapshot();
    expect(tokenize('"hello $((((name))))"')).toMatchSnapshot();
    expect(
      tokenize('"hello $("another $(interpolation), nice")"')
    ).toMatchSnapshot();
  });
});
