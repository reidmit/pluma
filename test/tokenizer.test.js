import { tokenTypes } from '../src/constants';
import { tokenize } from '../src/tokenizer';

const expectTokens = (input, output) => expect(tokenize(input)).toEqual(output);

describe('tokenizer', () => {
  describe('isolated token types', () => {
    test('boolean literals', () => {
      expectTokens(
        `
      true
      false
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
            lineStart: 3,
            lineEnd: 3,
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
        'this is a string  '
        'this is a
            multiline string'
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
        'this is an \${ 'interpolated' } string'
        'strings can have \${'interpolations \${'within'} interpolations'}, too!'
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
      const
      let
      import export
      `,
        [
          {
            type: tokenTypes.KEYWORD,
            value: 'const',
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
          }
        ]
      );
    });
  });
});
