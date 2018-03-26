import { tokenTypes } from '../../src/language/constants';
import tokenize from '../../src/language/tokenizer';

const expectTokens = (input, output) =>
  expect(tokenize({ source: input })).toEqual(output);

describe('tokenizer', () => {
  describe('isolated token types', () => {
    test('boolean literals', () => {
      expectTokens(
        `
      True

      False
      `,
        [
          {
            type: tokenTypes.BOOLEAN,
            value: true,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 10
          },
          {
            type: tokenTypes.BOOLEAN,
            value: false,
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 11
          }
        ]
      );
    });

    test('number literals (decimal)', () => {
      expectTokens(
        `
      47
      100
      23.3
      0
      `,
        [
          {
            type: tokenTypes.NUMBER,
            value: 47,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 8
          },
          {
            type: tokenTypes.NUMBER,
            value: 100,
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 9
          },
          {
            type: tokenTypes.NUMBER,
            value: 23.3,
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 10
          },
          {
            type: tokenTypes.NUMBER,
            value: 0,
            lineStart: 5,
            lineEnd: 5,
            columnStart: 6,
            columnEnd: 7
          }
        ]
      );
    });

    test('number literals (binary)', () => {
      expectTokens(
        `
      0b01010
      0B1110
      0b101
      `,
        [
          {
            type: tokenTypes.NUMBER,
            value: 0b01010,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 13
          },
          {
            type: tokenTypes.NUMBER,
            value: 0b1110,
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 12
          },
          {
            type: tokenTypes.NUMBER,
            value: 0b101,
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 11
          }
        ]
      );
    });

    test('number literals (octal)', () => {
      expectTokens(
        `
      0o1472322651
      0o02
      0O20
      `,
        [
          {
            type: tokenTypes.NUMBER,
            value: 0o1472322651,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 18
          },
          {
            type: tokenTypes.NUMBER,
            value: 0o02,
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 10
          },
          {
            type: tokenTypes.NUMBER,
            value: 0o20,
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 10
          }
        ]
      );
    });

    test('number literals (hex)', () => {
      expectTokens(
        `
      0xdeadBEEF
      0x0123456789abcdefABCDEF
      0X100
      0xFACADE
      0xeffaced
      `,
        [
          {
            type: tokenTypes.NUMBER,
            value: 0xdeadbeef,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 16
          },
          {
            type: tokenTypes.NUMBER,
            value: 0x0123456789abcdefabcdef,
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 30
          },
          {
            type: tokenTypes.NUMBER,
            value: 0x100,
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 11
          },
          {
            type: tokenTypes.NUMBER,
            value: 0xfacade,
            lineStart: 5,
            lineEnd: 5,
            columnStart: 6,
            columnEnd: 14
          },
          {
            type: tokenTypes.NUMBER,
            value: 0xeffaced,
            lineStart: 6,
            lineEnd: 6,
            columnStart: 6,
            columnEnd: 15
          }
        ]
      );
    });

    test('number literals (special)', () => {
      expectTokens(
        `
      NaN
      Infinity
      `,
        [
          {
            type: tokenTypes.NUMBER,
            value: NaN,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 9
          },
          {
            type: tokenTypes.NUMBER,
            value: Infinity,
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 14
          }
        ]
      );
    });

    test('string literals', () => {
      expectTokens(
        `
        "this is a string  "
        "this is a
            multiline string"
      `,
        [
          {
            type: tokenTypes.STRING,
            value: 'this is a string  ',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 8,
            columnEnd: 28
          },
          {
            type: tokenTypes.STRING,
            value: 'this is a\n            multiline string',
            lineStart: 3,
            lineEnd: 4,
            columnStart: 8,
            columnEnd: 29
          }
        ]
      );
    });

    test('string literals (interpolated)', () => {
      expectTokens(
        `
        "this is an \${ "interpolated" } string"
        "strings can have \${"interpolations \${"within"} interpolations"}, too!"
      `,
        [
          {
            type: tokenTypes.STRING,
            value: 'this is an ',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 8,
            columnEnd: 20
          },
          {
            type: tokenTypes.SYMBOL,
            value: '${',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 20,
            columnEnd: 22
          },
          {
            type: tokenTypes.STRING,
            value: 'interpolated',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 23,
            columnEnd: 37
          },
          {
            type: tokenTypes.SYMBOL,
            value: '}',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 38,
            columnEnd: 39
          },
          {
            type: tokenTypes.STRING,
            value: ' string',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 39,
            columnEnd: 47
          },
          {
            type: tokenTypes.STRING,
            value: 'strings can have ',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 8,
            columnEnd: 26
          },
          {
            type: tokenTypes.SYMBOL,
            value: '${',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 26,
            columnEnd: 28
          },
          {
            type: tokenTypes.STRING,
            value: 'interpolations ',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 28,
            columnEnd: 44
          },
          {
            type: tokenTypes.SYMBOL,
            value: '${',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 44,
            columnEnd: 46
          },
          {
            type: tokenTypes.STRING,
            value: 'within',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 46,
            columnEnd: 54
          },
          {
            type: tokenTypes.SYMBOL,
            value: '}',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 54,
            columnEnd: 55
          },
          {
            type: tokenTypes.STRING,
            value: ' interpolations',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 55,
            columnEnd: 71
          },
          {
            type: tokenTypes.SYMBOL,
            value: '}',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 71,
            columnEnd: 72
          },
          {
            type: tokenTypes.STRING,
            value: ', too!',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 72,
            columnEnd: 79
          }
        ]
      );
    });

    test('null literal', () => {
      expectTokens(
        `
      null
      `,
        [
          {
            type: tokenTypes.NULL,
            value: null,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 10
          }
        ]
      );
    });

    test('undefined literal', () => {
      expectTokens(
        `
      undefined
      `,
        [
          {
            type: tokenTypes.UNDEFINED,
            value: undefined,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 15
          }
        ]
      );
    });

    test('keywords', () => {
      expectTokens(
        `
      async
      let
      import export
      `,
        [
          {
            type: tokenTypes.KEYWORD,
            value: 'async',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 11
          },
          {
            type: tokenTypes.KEYWORD,
            value: 'let',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 9
          },
          {
            type: tokenTypes.KEYWORD,
            value: 'import',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 12
          },
          {
            type: tokenTypes.KEYWORD,
            value: 'export',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 13,
            columnEnd: 19
          }
        ]
      );
    });

    test('symbols', () => {
      expectTokens(
        ` {} \${ #
      , => =
      ( . ) :
      ][
        ...
      `,
        [
          {
            type: tokenTypes.SYMBOL,
            value: '{',
            lineStart: 1,
            lineEnd: 1,
            columnStart: 1,
            columnEnd: 2
          },
          {
            type: tokenTypes.SYMBOL,
            value: '}',
            lineStart: 1,
            lineEnd: 1,
            columnStart: 2,
            columnEnd: 3
          },
          {
            type: tokenTypes.SYMBOL,
            value: '${',
            lineStart: 1,
            lineEnd: 1,
            columnStart: 4,
            columnEnd: 6
          },
          {
            type: tokenTypes.SYMBOL,
            value: '#',
            lineStart: 1,
            lineEnd: 1,
            columnStart: 7,
            columnEnd: 8
          },
          {
            type: tokenTypes.SYMBOL,
            value: ',',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 6,
            columnEnd: 7
          },
          {
            type: tokenTypes.SYMBOL,
            value: '=>',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 8,
            columnEnd: 10
          },
          {
            type: tokenTypes.SYMBOL,
            value: '=',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 11,
            columnEnd: 12
          },
          {
            type: tokenTypes.SYMBOL,
            value: '(',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 6,
            columnEnd: 7
          },
          {
            type: tokenTypes.SYMBOL,
            value: '.',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 8,
            columnEnd: 9
          },
          {
            type: tokenTypes.SYMBOL,
            value: ')',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 10,
            columnEnd: 11
          },
          {
            type: tokenTypes.SYMBOL,
            value: ':',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 12,
            columnEnd: 13
          },
          {
            type: tokenTypes.SYMBOL,
            value: ']',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 6,
            columnEnd: 7
          },
          {
            type: tokenTypes.SYMBOL,
            value: '[',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 7,
            columnEnd: 8
          },
          {
            type: tokenTypes.SYMBOL,
            value: '...',
            lineStart: 5,
            lineEnd: 5,
            columnStart: 8,
            columnEnd: 11
          }
        ]
      );
    });

    test('identifiers', () => {
      expectTokens(
        `
        hello WORLD
          _someToken$3`,
        [
          {
            type: tokenTypes.IDENTIFIER,
            value: 'hello',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 8,
            columnEnd: 13
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'WORLD',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 14,
            columnEnd: 19
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: '_someToken$3',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 10,
            columnEnd: 22
          }
        ]
      );
    });

    test('single values', () => {
      expectTokens('47', [
        {
          type: tokenTypes.NUMBER,
          value: 47,
          lineStart: 1,
          lineEnd: 1,
          columnStart: 0,
          columnEnd: 2
        }
      ]);
    });
  });

  describe('mixed token types', () => {
    test('let statements', () => {
      expectTokens(
        `
        let x = 47
        let funky = a => "hello there,
          \${a}!"
        `,
        [
          {
            type: tokenTypes.KEYWORD,
            value: 'let',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 8,
            columnEnd: 11
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'x',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 12,
            columnEnd: 13
          },
          {
            type: tokenTypes.SYMBOL,
            value: '=',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 14,
            columnEnd: 15
          },
          {
            type: tokenTypes.NUMBER,
            value: 47,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 16,
            columnEnd: 18
          },
          {
            type: tokenTypes.KEYWORD,
            value: 'let',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 8,
            columnEnd: 11
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'funky',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 12,
            columnEnd: 17
          },
          {
            type: tokenTypes.SYMBOL,
            value: '=',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 18,
            columnEnd: 19
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'a',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 20,
            columnEnd: 21
          },
          {
            type: tokenTypes.SYMBOL,
            value: '=>',
            lineStart: 3,
            lineEnd: 3,
            columnStart: 22,
            columnEnd: 24
          },
          {
            type: tokenTypes.STRING,
            value: 'hello there,\n          ',
            lineStart: 3,
            lineEnd: 4,
            columnStart: 25,
            columnEnd: 10
          },
          {
            type: tokenTypes.SYMBOL,
            value: '${',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 10,
            columnEnd: 12
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'a',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 12,
            columnEnd: 13
          },
          {
            type: tokenTypes.SYMBOL,
            value: '}',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 13,
            columnEnd: 14
          },
          {
            type: tokenTypes.STRING,
            value: '!',
            lineStart: 4,
            lineEnd: 4,
            columnStart: 14,
            columnEnd: 16
          }
        ]
      );
    });

    test('member expressions', () => {
      expectTokens(
        `
        a.b[c]["test"][47]
      `,
        [
          {
            type: tokenTypes.IDENTIFIER,
            value: 'a',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 8,
            columnEnd: 9
          },
          {
            type: tokenTypes.SYMBOL,
            value: '.',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 9,
            columnEnd: 10
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'b',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 10,
            columnEnd: 11
          },
          {
            type: tokenTypes.SYMBOL,
            value: '[',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 11,
            columnEnd: 12
          },
          {
            type: tokenTypes.IDENTIFIER,
            value: 'c',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 12,
            columnEnd: 13
          },
          {
            type: tokenTypes.SYMBOL,
            value: ']',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 13,
            columnEnd: 14
          },
          {
            type: tokenTypes.SYMBOL,
            value: '[',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 14,
            columnEnd: 15
          },
          {
            type: tokenTypes.STRING,
            value: 'test',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 15,
            columnEnd: 21
          },
          {
            type: tokenTypes.SYMBOL,
            value: ']',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 21,
            columnEnd: 22
          },
          {
            type: tokenTypes.SYMBOL,
            value: '[',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 22,
            columnEnd: 23
          },
          {
            type: tokenTypes.NUMBER,
            value: 47,
            lineStart: 2,
            lineEnd: 2,
            columnStart: 23,
            columnEnd: 25
          },
          {
            type: tokenTypes.SYMBOL,
            value: ']',
            lineStart: 2,
            lineEnd: 2,
            columnStart: 25,
            columnEnd: 26
          }
        ]
      );
    });
  });

  describe('error cases', () => {
    test('unrecognized tokens', () => {
      const tokens = tokenize({
        source: `let a = 100
let b = 200
let c = 300
let d = 400
let e = 600

let f = 700
let x = 47
hello & %
let fn = z => "hello"
let y = "test"
let z = "test"
`
      });

      console.log({ tokens });
    });
  });
});
