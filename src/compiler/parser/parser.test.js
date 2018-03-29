import parse from './parser';
import tokenize from '../tokenizer';
import { nodeTypes } from '../constants';

const expectParseResult = ({ source, lineStart, lineEnd, body }) => {
  const tokens = tokenize({ source });
  expect(parse({ source, tokens })).toEqual({
    type: nodeTypes.MODULE,
    lineStart,
    lineEnd,
    body
  });
};

const expectParseError = (source, errorMessageRegex) => {
  const tokens = tokenize({ source });
  let error;

  try {
    parse({ source, tokens });
  } catch (err) {
    error = err;
  }

  expect(error).toBeDefined();
  expect(error.name).toBe('Parser error');
  expect(error.message).toMatch(errorMessageRegex);
};

describe('parser', () => {
  describe('valid programs', () => {
    test('empty program', () => {
      expectParseResult({
        source: '',
        lineStart: 1,
        lineEnd: 1,
        body: []
      });
    });

    test('number literal', () => {
      expectParseResult({
        source: '47',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.NUMBER,
            value: 47,
            lineStart: 1,
            lineEnd: 1
          }
        ]
      });
    });

    test('boolean literal', () => {
      expectParseResult({
        source: 'True',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.BOOLEAN,
            value: true,
            lineStart: 1,
            lineEnd: 1
          }
        ]
      });
    });

    test('identifier', () => {
      expectParseResult({
        source: 'lol',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.IDENTIFIER,
            value: 'lol',
            lineStart: 1,
            lineEnd: 1
          }
        ]
      });
    });

    test('string literal', () => {
      expectParseResult({
        source: '"hello, world!"',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.STRING,
            value: 'hello, world!',
            lineStart: 1,
            lineEnd: 1
          }
        ]
      });
    });

    test('interpolated string literal', () => {
      expectParseResult({
        source: '"hello, ${name}!"',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.INTERPOLATED_STRING,
            lineStart: 1,
            lineEnd: 1,
            literals: [
              {
                type: nodeTypes.STRING,
                value: 'hello, ',
                lineStart: 1,
                lineEnd: 1
              },
              {
                type: nodeTypes.STRING,
                value: '!',
                lineStart: 1,
                lineEnd: 1
              }
            ],
            expressions: [
              {
                type: nodeTypes.IDENTIFIER,
                value: 'name',
                lineStart: 1,
                lineEnd: 1
              }
            ]
          }
        ]
      });
    });

    test('member expression with dots (non-computed)', () => {
      expectParseResult({
        source: 'a.b.c',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.MEMBER_EXPRESSION,
            lineStart: 1,
            lineEnd: 1,
            identifiers: [
              {
                type: nodeTypes.IDENTIFIER,
                value: 'a',
                lineStart: 1,
                lineEnd: 1
              },
              {
                type: nodeTypes.IDENTIFIER,
                value: 'b',
                lineStart: 1,
                lineEnd: 1
              },
              {
                type: nodeTypes.IDENTIFIER,
                value: 'c',
                lineStart: 1,
                lineEnd: 1
              }
            ]
          }
        ]
      });
    });

    test('function (one param)', () => {
      expectParseResult({
        source: 'x => 47',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.FUNCTION,
            lineStart: 1,
            lineEnd: 1,
            parameter: {
              type: nodeTypes.IDENTIFIER,
              value: 'x',
              lineStart: 1,
              lineEnd: 1
            },
            body: {
              type: nodeTypes.NUMBER,
              value: 47,
              lineStart: 1,
              lineEnd: 1
            }
          }
        ]
      });
    });

    test('function (two params)', () => {
      expectParseResult({
        source: 'x => y => 47',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.FUNCTION,
            lineStart: 1,
            lineEnd: 1,
            parameter: {
              type: nodeTypes.IDENTIFIER,
              value: 'x',
              lineStart: 1,
              lineEnd: 1
            },
            body: {
              type: nodeTypes.FUNCTION,
              lineStart: 1,
              lineEnd: 1,
              parameter: {
                type: nodeTypes.IDENTIFIER,
                value: 'y',
                lineStart: 1,
                lineEnd: 1
              },
              body: {
                type: nodeTypes.NUMBER,
                value: 47,
                lineStart: 1,
                lineEnd: 1
              }
            }
          }
        ]
      });
    });

    test('assignment', () => {
      expectParseResult({
        source: `
          let hello = 47
          let someString = "hello, world!"
        `,
        lineStart: 2,
        lineEnd: 3,
        body: [
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 2,
            lineEnd: 2,
            leftSide: {
              type: nodeTypes.IDENTIFIER,
              value: 'hello',
              lineStart: 2,
              lineEnd: 2
            },
            rightSide: {
              type: nodeTypes.NUMBER,
              value: 47,
              lineStart: 2,
              lineEnd: 2
            }
          },
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 3,
            lineEnd: 3,
            leftSide: {
              type: nodeTypes.IDENTIFIER,
              value: 'someString',
              lineStart: 3,
              lineEnd: 3
            },
            rightSide: {
              type: nodeTypes.STRING,
              value: 'hello, world!',
              lineStart: 3,
              lineEnd: 3
            }
          }
        ]
      });
    });

    test('call expression (getter function)', () => {
      expectParseResult({
        source: '.someProp someObject',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.CALL,
            lineStart: 1,
            lineEnd: 1,
            callee: {
              type: nodeTypes.IDENTIFIER,
              value: 'someProp',
              isGetter: true,
              isSetter: false,
              lineStart: 1,
              lineEnd: 1
            },
            arg: {
              type: nodeTypes.IDENTIFIER,
              value: 'someObject',
              lineStart: 1,
              lineEnd: 1
            }
          }
        ]
      });
    });

    test('call expression (setter function)', () => {
      expectParseResult({
        source: '@someProp 47 someObject',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.CALL,
            lineStart: 1,
            lineEnd: 1,
            callee: {
              type: nodeTypes.CALL,
              lineStart: 1,
              lineEnd: 1,
              callee: {
                type: nodeTypes.IDENTIFIER,
                value: 'someProp',
                isGetter: false,
                isSetter: true,
                lineStart: 1,
                lineEnd: 1
              },
              arg: {
                type: nodeTypes.NUMBER,
                value: 47,
                lineStart: 1,
                lineEnd: 1
              }
            },
            arg: {
              type: nodeTypes.IDENTIFIER,
              value: 'someObject',
              lineStart: 1,
              lineEnd: 1
            }
          }
        ]
      });
    });

    test('call expression (single argument)', () => {
      expectParseResult({
        source: 'someFunc someArg',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.CALL,
            lineStart: 1,
            lineEnd: 1,
            callee: {
              type: nodeTypes.IDENTIFIER,
              value: 'someFunc',
              lineStart: 1,
              lineEnd: 1
            },
            arg: {
              type: nodeTypes.IDENTIFIER,
              value: 'someArg',
              lineStart: 1,
              lineEnd: 1
            }
          }
        ]
      });
    });

    test('call expression (multiple arguments)', () => {
      expectParseResult({
        source: 'helloWorld 47 "something here" cool',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.CALL,
            lineStart: 1,
            lineEnd: 1,
            callee: {
              type: nodeTypes.CALL,
              lineStart: 1,
              lineEnd: 1,
              callee: {
                type: nodeTypes.CALL,
                lineStart: 1,
                lineEnd: 1,
                callee: {
                  type: nodeTypes.IDENTIFIER,
                  value: 'helloWorld',
                  lineStart: 1,
                  lineEnd: 1
                },
                arg: {
                  type: nodeTypes.NUMBER,
                  value: 47,
                  lineStart: 1,
                  lineEnd: 1
                }
              },
              arg: {
                type: nodeTypes.STRING,
                value: 'something here',
                lineStart: 1,
                lineEnd: 1
              }
            },
            arg: {
              type: nodeTypes.IDENTIFIER,
              value: 'cool',
              lineStart: 1,
              lineEnd: 1
            }
          }
        ]
      });
    });

    test('nested call expressions with parentheses', () => {
      expectParseResult({
        source: `
          someFunc (someOtherFunc 3) 4
        `,
        lineStart: 2,
        lineEnd: 2,
        body: [
          {
            type: nodeTypes.CALL,
            lineStart: 2,
            lineEnd: 2,
            callee: {
              type: nodeTypes.CALL,
              lineStart: 2,
              lineEnd: 2,
              callee: {
                type: nodeTypes.IDENTIFIER,
                value: 'someFunc',
                lineStart: 2,
                lineEnd: 2
              },
              arg: {
                type: nodeTypes.CALL,
                lineStart: 2,
                lineEnd: 2,
                callee: {
                  type: nodeTypes.IDENTIFIER,
                  value: 'someOtherFunc',
                  lineStart: 2,
                  lineEnd: 2
                },
                arg: {
                  type: nodeTypes.NUMBER,
                  value: 3,
                  lineStart: 2,
                  lineEnd: 2
                }
              }
            },
            arg: {
              type: nodeTypes.NUMBER,
              value: 4,
              lineStart: 2,
              lineEnd: 2
            }
          }
        ]
      });
    });

    test('array expressions (empty)', () => {
      expectParseResult({
        source: '[]',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.ARRAY,
            lineStart: 1,
            lineEnd: 1,
            elements: []
          }
        ]
      });
    });

    test('array expressions (basic)', () => {
      expectParseResult({
        source: `[
          1, test, True, "hello"
        ]
        `,
        lineStart: 1,
        lineEnd: 3,
        body: [
          {
            type: nodeTypes.ARRAY,
            lineStart: 1,
            lineEnd: 3,
            elements: [
              {
                type: nodeTypes.NUMBER,
                value: 1,
                lineStart: 2,
                lineEnd: 2
              },
              {
                type: nodeTypes.IDENTIFIER,
                value: 'test',
                lineStart: 2,
                lineEnd: 2
              },
              {
                type: nodeTypes.BOOLEAN,
                value: true,
                lineStart: 2,
                lineEnd: 2
              },
              {
                type: nodeTypes.STRING,
                value: 'hello',
                lineStart: 2,
                lineEnd: 2
              }
            ]
          }
        ]
      });
    });

    test('object expressions (empty)', () => {
      expectParseResult({
        source: '{}',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.OBJECT,
            lineStart: 1,
            lineEnd: 1,
            properties: []
          }
        ]
      });
    });

    test('object expressions (basic)', () => {
      expectParseResult({
        source: '{a: 1, b: "hello", c-d: True}',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.OBJECT,
            lineStart: 1,
            lineEnd: 1,
            properties: [
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'a'
                },
                value: {
                  type: nodeTypes.NUMBER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 1
                }
              },
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'b'
                },
                value: {
                  type: nodeTypes.STRING,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'hello'
                }
              },
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'c-d'
                },
                value: {
                  type: nodeTypes.BOOLEAN,
                  lineStart: 1,
                  lineEnd: 1,
                  value: true
                }
              }
            ]
          }
        ]
      });
    });

    test('object expressions (shorthand keys)', () => {
      expectParseResult({
        source: '{ short, hand }',
        lineStart: 1,
        lineEnd: 1,
        body: [
          {
            type: nodeTypes.OBJECT,
            lineStart: 1,
            lineEnd: 1,
            properties: [
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'short'
                },
                value: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'short'
                }
              },
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'hand'
                },
                value: {
                  type: nodeTypes.IDENTIFIER,
                  lineStart: 1,
                  lineEnd: 1,
                  value: 'hand'
                }
              }
            ]
          }
        ]
      });
    });

    test('tuples', () => {
      expectParseResult({
        source: `
          (1, True, "hello", nice)
          (3, 4,
            5)
        `,
        lineStart: 2,
        lineEnd: 4,
        body: [
          {
            type: nodeTypes.TUPLE,
            lineStart: 2,
            lineEnd: 2,
            entries: [
              {
                type: nodeTypes.NUMBER,
                value: 1,
                lineStart: 2,
                lineEnd: 2
              },
              {
                type: nodeTypes.BOOLEAN,
                value: true,
                lineStart: 2,
                lineEnd: 2
              },
              {
                type: nodeTypes.STRING,
                value: 'hello',
                lineStart: 2,
                lineEnd: 2
              },
              {
                type: nodeTypes.IDENTIFIER,
                value: 'nice',
                lineStart: 2,
                lineEnd: 2
              }
            ]
          },
          {
            type: nodeTypes.TUPLE,
            lineStart: 3,
            lineEnd: 4,
            entries: [
              {
                type: nodeTypes.NUMBER,
                value: 3,
                lineStart: 3,
                lineEnd: 3
              },
              {
                type: nodeTypes.NUMBER,
                value: 4,
                lineStart: 3,
                lineEnd: 3
              },
              {
                type: nodeTypes.NUMBER,
                value: 5,
                lineStart: 4,
                lineEnd: 4
              }
            ]
          }
        ]
      });
    });

    test('if-then-else expressions', () => {
      expectParseResult({
        source: `
          if True
            then 47
            else 100

          if (and True False) then "no" else "yes"

          if False then "okay"
            else if True then "maybe"
              else "nah"
        `,
        lineStart: 2,
        lineEnd: 10,
        body: [
          {
            type: nodeTypes.CONDITIONAL,
            lineStart: 2,
            lineEnd: 4,
            predicate: {
              type: nodeTypes.BOOLEAN,
              value: true,
              lineStart: 2,
              lineEnd: 2
            },
            thenCase: {
              type: nodeTypes.NUMBER,
              value: 47,
              lineStart: 3,
              lineEnd: 3
            },
            elseCase: {
              type: nodeTypes.NUMBER,
              value: 100,
              lineStart: 4,
              lineEnd: 4
            }
          },
          {
            type: nodeTypes.CONDITIONAL,
            lineStart: 6,
            lineEnd: 6,
            predicate: {
              type: nodeTypes.CALL,
              lineStart: 6,
              lineEnd: 6,
              callee: {
                type: nodeTypes.CALL,
                lineStart: 6,
                lineEnd: 6,
                callee: {
                  type: nodeTypes.IDENTIFIER,
                  value: 'and',
                  lineStart: 6,
                  lineEnd: 6
                },
                arg: {
                  type: nodeTypes.BOOLEAN,
                  value: true,
                  lineStart: 6,
                  lineEnd: 6
                }
              },
              arg: {
                type: nodeTypes.BOOLEAN,
                value: false,
                lineStart: 6,
                lineEnd: 6
              }
            },
            thenCase: {
              type: nodeTypes.STRING,
              value: 'no',
              lineStart: 6,
              lineEnd: 6
            },
            elseCase: {
              type: nodeTypes.STRING,
              value: 'yes',
              lineStart: 6,
              lineEnd: 6
            }
          },
          {
            type: nodeTypes.CONDITIONAL,
            lineStart: 8,
            lineEnd: 10,
            predicate: {
              type: nodeTypes.BOOLEAN,
              value: false,
              lineStart: 8,
              lineEnd: 8
            },
            thenCase: {
              type: nodeTypes.STRING,
              value: 'okay',
              lineStart: 8,
              lineEnd: 8
            },
            elseCase: {
              type: nodeTypes.CONDITIONAL,
              lineStart: 9,
              lineEnd: 10,
              predicate: {
                type: nodeTypes.BOOLEAN,
                value: true,
                lineStart: 9,
                lineEnd: 9
              },
              thenCase: {
                type: nodeTypes.STRING,
                value: 'maybe',
                lineStart: 9,
                lineEnd: 9
              },
              elseCase: {
                type: nodeTypes.STRING,
                value: 'nah',
                lineStart: 10,
                lineEnd: 10
              }
            }
          }
        ]
      });
    });

    test('multiple complex assignments', () => {
      expectParseResult({
        source: `
          let func1 = helloWorld 47 "something here" cool
          let func2 = func1 True
        `,
        lineStart: 2,
        lineEnd: 3,
        body: [
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 2,
            lineEnd: 2,
            leftSide: {
              type: nodeTypes.IDENTIFIER,
              value: 'func1',
              lineStart: 2,
              lineEnd: 2
            },
            rightSide: {
              type: nodeTypes.CALL,
              lineStart: 2,
              lineEnd: 2,
              callee: {
                type: nodeTypes.CALL,
                lineStart: 2,
                lineEnd: 2,
                callee: {
                  type: nodeTypes.CALL,
                  lineStart: 2,
                  lineEnd: 2,
                  callee: {
                    type: nodeTypes.IDENTIFIER,
                    value: 'helloWorld',
                    lineStart: 2,
                    lineEnd: 2
                  },
                  arg: {
                    type: nodeTypes.NUMBER,
                    value: 47,
                    lineStart: 2,
                    lineEnd: 2
                  }
                },
                arg: {
                  type: nodeTypes.STRING,
                  value: 'something here',
                  lineStart: 2,
                  lineEnd: 2
                }
              },
              arg: {
                type: nodeTypes.IDENTIFIER,
                value: 'cool',
                lineStart: 2,
                lineEnd: 2
              }
            }
          },
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 3,
            lineEnd: 3,
            leftSide: {
              type: nodeTypes.IDENTIFIER,
              value: 'func2',
              lineStart: 3,
              lineEnd: 3
            },
            rightSide: {
              type: nodeTypes.CALL,
              lineStart: 3,
              lineEnd: 3,
              callee: {
                type: nodeTypes.IDENTIFIER,
                value: 'func1',
                lineStart: 3,
                lineEnd: 3
              },
              arg: {
                type: nodeTypes.BOOLEAN,
                value: true,
                lineStart: 3,
                lineEnd: 3
              }
            }
          }
        ]
      });
    });

    test('function definition followed by call', () => {
      expectParseResult({
        source: `
          let fn = a => "hello, world!"

          fn 1
        `,
        lineStart: 2,
        lineEnd: 4,
        body: [
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 2,
            lineEnd: 2,
            leftSide: {
              type: nodeTypes.IDENTIFIER,
              value: 'fn',
              lineStart: 2,
              lineEnd: 2
            },
            rightSide: {
              type: nodeTypes.FUNCTION,
              lineStart: 2,
              lineEnd: 2,
              parameter: {
                type: nodeTypes.IDENTIFIER,
                value: 'a',
                lineStart: 2,
                lineEnd: 2
              },
              body: {
                type: nodeTypes.STRING,
                value: 'hello, world!',
                lineStart: 2,
                lineEnd: 2
              }
            }
          },
          {
            type: nodeTypes.CALL,
            lineStart: 4,
            lineEnd: 4,
            callee: {
              type: nodeTypes.IDENTIFIER,
              value: 'fn',
              lineStart: 4,
              lineEnd: 4
            },
            arg: {
              type: nodeTypes.NUMBER,
              value: 1,
              lineStart: 4,
              lineEnd: 4
            }
          }
        ]
      });
    });

    test('function definition followed by array', () => {
      expectParseResult({
        source: `
          let toStr = s => fn s

          [1, 2, 3]
        `,
        lineStart: 2,
        lineEnd: 4,
        body: [
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 2,
            lineEnd: 2,
            leftSide: {
              type: nodeTypes.IDENTIFIER,
              value: 'toStr',
              lineStart: 2,
              lineEnd: 2
            },
            rightSide: {
              type: nodeTypes.FUNCTION,
              lineStart: 2,
              lineEnd: 2,
              parameter: {
                type: nodeTypes.IDENTIFIER,
                value: 's',
                lineStart: 2,
                lineEnd: 2
              },
              body: {
                type: nodeTypes.CALL,
                lineStart: 2,
                lineEnd: 2,
                callee: {
                  type: nodeTypes.IDENTIFIER,
                  value: 'fn',
                  lineStart: 2,
                  lineEnd: 2
                },
                arg: {
                  type: nodeTypes.IDENTIFIER,
                  value: 's',
                  lineStart: 2,
                  lineEnd: 2
                }
              }
            }
          },
          {
            type: nodeTypes.ARRAY,
            lineStart: 4,
            lineEnd: 4,
            elements: [
              {
                type: nodeTypes.NUMBER,
                value: 1,
                lineStart: 4,
                lineEnd: 4
              },
              {
                type: nodeTypes.NUMBER,
                value: 2,
                lineStart: 4,
                lineEnd: 4
              },
              {
                type: nodeTypes.NUMBER,
                value: 3,
                lineStart: 4,
                lineEnd: 4
              }
            ]
          }
        ]
      });
    });
  });

  describe('error cases', () => {
    test('unexpected token after "let"', () => {
      expectParseError(
        'let let',
        'Unexpected keyword "let" found after "let" keyword. Expected an identifier.'
      );
    });

    test('unexpected end of input', () => {
      expectParseError('(', 'Unexpectedly reached end of input.');
    });

    test('unclosed parentheses', () => {
      expectParseError(
        '(47 "hello"',
        'Missing closing ")" to match opening "(" at line 1, column 0.'
      );
    });

    test('unclosed array brackets', () => {
      expectParseError(
        'fn [1, 2, 3',
        'Missing closing "]" to match opening "[" at line 1, column 3.'
      );
    });
  });
});
