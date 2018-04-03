import parse from './parser';
import tokenize from '../tokenizer';
import { nodeTypes } from '../constants';
import { buildNode } from '../ast-nodes';

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
  expect(error.name).toMatch(/^Syntax error at/);
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
        body: [buildNode.Number(1, 1)({ value: 47 })]
      });
    });

    test('boolean literal', () => {
      expectParseResult({
        source: 'True',
        lineStart: 1,
        lineEnd: 1,
        body: [buildNode.Boolean(1, 1)({ value: true })]
      });
    });

    test('identifier', () => {
      expectParseResult({
        source: 'lol',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Identifier(1, 1)({
            value: 'lol',
            isGetter: false,
            isSetter: false
          })
        ]
      });
    });

    test('string literal', () => {
      expectParseResult({
        source: '"hello, world!"',
        lineStart: 1,
        lineEnd: 1,
        body: [buildNode.String(1, 1)({ value: 'hello, world!' })]
      });
    });

    test('interpolated string literal', () => {
      expectParseResult({
        source: '"hello, ${name}!"',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.InterpolatedString(1, 1)({
            literals: [
              buildNode.String(1, 1)({ value: 'hello, ' }),
              buildNode.String(1, 1)({ value: '!' })
            ],
            expressions: [
              buildNode.Identifier(1, 1)({
                value: 'name',
                isGetter: false,
                isSetter: false
              })
            ]
          })
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
            parts: [
              buildNode.Identifier(1, 1)({
                value: 'a',
                isGetter: false,
                isSetter: false
              }),
              buildNode.Identifier(1, 1)({
                value: 'b',
                isGetter: false,
                isSetter: false
              }),
              buildNode.Identifier(1, 1)({
                value: 'c',
                isGetter: false,
                isSetter: false
              })
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
            parameter: buildNode.Identifier(1, 1)({
              value: 'x',
              isGetter: false,
              isSetter: false
            }),
            body: buildNode.Number(1, 1)({ value: 47 })
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
            parameter: buildNode.Identifier(1, 1)({
              value: 'x',
              isGetter: false,
              isSetter: false
            }),
            body: {
              type: nodeTypes.FUNCTION,
              lineStart: 1,
              lineEnd: 1,
              parameter: buildNode.Identifier(1, 1)({
                value: 'y',
                isGetter: false,
                isSetter: false
              }),
              body: buildNode.Number(1, 1)({ value: 47 })
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
            comments: [],
            lineStart: 2,
            lineEnd: 2,
            leftSide: buildNode.Identifier(2, 2)({
              value: 'hello',
              isGetter: false,
              isSetter: false
            }),
            rightSide: buildNode.Number(2, 2)({ value: 47 })
          },
          {
            type: nodeTypes.ASSIGNMENT,
            comments: [],
            lineStart: 3,
            lineEnd: 3,
            leftSide: buildNode.Identifier(3, 3)({
              value: 'someString',
              isGetter: false,
              isSetter: false
            }),
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
            callee: buildNode.Identifier(1, 1)({
              value: 'someProp',
              isGetter: true,
              isSetter: false
            }),
            arg: buildNode.Identifier(1, 1)({
              value: 'someObject',
              isGetter: false,
              isSetter: false
            })
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
              callee: buildNode.Identifier(1, 1)({
                value: 'someProp',
                isGetter: false,
                isSetter: true
              }),
              arg: buildNode.Number(1, 1)({ value: 47 })
            },
            arg: buildNode.Identifier(1, 1)({
              value: 'someObject',
              isGetter: false,
              isSetter: false
            })
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
            callee: buildNode.Identifier(1, 1)({
              value: 'someFunc',
              isGetter: false,
              isSetter: false
            }),
            arg: buildNode.Identifier(1, 1)({
              value: 'someArg',
              isGetter: false,
              isSetter: false
            })
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
                callee: buildNode.Identifier(1, 1)({
                  value: 'helloWorld',
                  isGetter: false,
                  isSetter: false
                }),
                arg: buildNode.Number(1, 1)({ value: 47 })
              },
              arg: {
                type: nodeTypes.STRING,
                value: 'something here',
                lineStart: 1,
                lineEnd: 1
              }
            },
            arg: buildNode.Identifier(1, 1)({
              value: 'cool',
              isGetter: false,
              isSetter: false
            })
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
              callee: buildNode.Identifier(2, 2)({
                value: 'someFunc',
                isGetter: false,
                isSetter: false
              }),
              arg: {
                type: nodeTypes.CALL,
                lineStart: 2,
                lineEnd: 2,
                callee: buildNode.Identifier(2, 2)({
                  value: 'someOtherFunc',
                  isGetter: false,
                  isSetter: false
                }),
                arg: buildNode.Number(2, 2)({ value: 3 })
              }
            },
            arg: buildNode.Number(2, 2)({ value: 4 })
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
              buildNode.Number(2, 2)({ value: 1 }),
              buildNode.Identifier(2, 2)({
                value: 'test',
                isGetter: false,
                isSetter: false
              }),
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
                key: buildNode.Identifier(1, 1)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                }),
                value: buildNode.Number(1, 1)({ value: 1 })
              },
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: buildNode.Identifier(1, 1)({
                  value: 'b',
                  isGetter: false,
                  isSetter: false
                }),
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
                key: buildNode.Identifier(1, 1)({
                  value: 'c-d',
                  isGetter: false,
                  isSetter: false
                }),
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
                key: buildNode.Identifier(1, 1)({
                  value: 'short',
                  isGetter: false,
                  isSetter: false
                }),
                value: buildNode.Identifier(1, 1)({
                  value: 'short',
                  isGetter: false,
                  isSetter: false
                })
              },
              {
                type: nodeTypes.OBJECT_PROPERTY,
                lineStart: 1,
                lineEnd: 1,
                key: buildNode.Identifier(1, 1)({
                  value: 'hand',
                  isGetter: false,
                  isSetter: false
                }),
                value: buildNode.Identifier(1, 1)({
                  value: 'hand',
                  isGetter: false,
                  isSetter: false
                })
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
              buildNode.Number(2, 2)({ value: 1 }),
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
              buildNode.Identifier(2, 2)({
                value: 'nice',
                isGetter: false,
                isSetter: false
              })
            ]
          },
          {
            type: nodeTypes.TUPLE,
            lineStart: 3,
            lineEnd: 4,
            entries: [
              buildNode.Number(3, 3)({ value: 3 }),
              buildNode.Number(3, 3)({ value: 4 }),
              buildNode.Number(4, 4)({ value: 5 })
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
            thenCase: buildNode.Number(3, 3)({ value: 47 }),
            elseCase: buildNode.Number(4, 4)({ value: 100 })
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
                callee: buildNode.Identifier(6, 6)({
                  value: 'and',
                  isGetter: false,
                  isSetter: false
                }),
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

    test('comments', () => {
      expectParseResult({
        source: `
          # This is a comment that
          # should be preserved for the below assignment
          let x = 47 # but not this

          # or this
        `,
        lineStart: 2,
        lineEnd: 4,
        body: [
          {
            type: nodeTypes.ASSIGNMENT,
            lineStart: 2,
            lineEnd: 4,
            comments: [
              ' This is a comment that',
              ' should be preserved for the below assignment'
            ],
            leftSide: buildNode.Identifier(4, 4)({
              value: 'x',
              isGetter: false,
              isSetter: false
            }),
            rightSide: buildNode.Number(4, 4)({ value: 47 })
          }
        ]
      });
    });

    test('type declarations', () => {
      expectParseResult({
        source: `
          type Letter = Alpha | Beta | Gamma
          type Maybe a =
              Just a
            | Nothing

          # Type declarations can have comments
          type Hello = World
        `,
        lineStart: 2,
        lineEnd: 8,
        body: [
          {
            type: nodeTypes.TYPE_DECLARATION,
            lineStart: 2,
            lineEnd: 2,
            comments: [],
            typeName: buildNode.Identifier(2, 2)({
              value: 'Letter',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeConstructors: [
              {
                type: nodeTypes.TYPE_CONSTRUCTOR,
                lineStart: 2,
                lineEnd: 2,
                typeName: buildNode.Identifier(2, 2)({
                  value: 'Alpha',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              },
              {
                type: nodeTypes.TYPE_CONSTRUCTOR,
                lineStart: 2,
                lineEnd: 2,
                typeName: buildNode.Identifier(2, 2)({
                  value: 'Beta',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              },
              {
                type: nodeTypes.TYPE_CONSTRUCTOR,
                lineStart: 2,
                lineEnd: 2,
                typeName: buildNode.Identifier(2, 2)({
                  value: 'Gamma',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              }
            ]
          },
          {
            type: nodeTypes.TYPE_DECLARATION,
            lineStart: 3,
            lineEnd: 5,
            comments: [],
            typeName: buildNode.Identifier(3, 3)({
              value: 'Maybe',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [
              buildNode.Identifier(3, 3)({
                value: 'a',
                isGetter: false,
                isSetter: false
              })
            ],
            typeConstructors: [
              {
                type: nodeTypes.TYPE_CONSTRUCTOR,
                lineStart: 4,
                lineEnd: 4,
                typeName: buildNode.Identifier(4, 4)({
                  value: 'Just',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: [
                  buildNode.Identifier(4, 4)({
                    value: 'a',
                    isGetter: false,
                    isSetter: false
                  })
                ]
              },
              {
                type: nodeTypes.TYPE_CONSTRUCTOR,
                lineStart: 5,
                lineEnd: 5,
                typeName: buildNode.Identifier(5, 5)({
                  value: 'Nothing',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              }
            ]
          },
          {
            type: nodeTypes.TYPE_DECLARATION,
            lineStart: 7,
            lineEnd: 8,
            comments: [' Type declarations can have comments'],
            typeName: buildNode.Identifier(8, 8)({
              value: 'Hello',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeConstructors: [
              {
                type: nodeTypes.TYPE_CONSTRUCTOR,
                lineStart: 8,
                lineEnd: 8,
                typeName: buildNode.Identifier(8, 8)({
                  value: 'World',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              }
            ]
          }
        ]
      });
    });

    test('type alias declarations (simple)', () => {
      expectParseResult({
        source: `
          type alias Hello = String
          type alias Test a = Something a
        `,
        lineStart: 2,
        lineEnd: 3,
        body: [
          {
            type: nodeTypes.TYPE_ALIAS_DECLARATION,
            lineStart: 2,
            lineEnd: 2,
            typeName: buildNode.Identifier(2, 2)({
              value: 'Hello',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: {
              type: nodeTypes.TYPE_TAG,
              lineStart: 2,
              lineEnd: 2,
              typeTagName: buildNode.Identifier(2, 2)({
                value: 'String',
                isGetter: false,
                isSetter: false
              }),
              typeExpression: null
            }
          },
          {
            type: nodeTypes.TYPE_ALIAS_DECLARATION,
            lineStart: 3,
            lineEnd: 3,
            typeName: buildNode.Identifier(3, 3)({
              value: 'Test',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [
              buildNode.Identifier(3, 3)({
                value: 'a',
                isGetter: false,
                isSetter: false
              })
            ],
            typeExpression: {
              type: nodeTypes.TYPE_TAG,
              lineStart: 3,
              lineEnd: 3,
              typeTagName: buildNode.Identifier(3, 3)({
                value: 'Something',
                isGetter: false,
                isSetter: false
              }),
              typeExpression: {
                type: nodeTypes.TYPE_VARIABLE,
                lineStart: 3,
                lineEnd: 3,
                typeName: buildNode.Identifier(3, 3)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                })
              }
            }
          }
        ]
      });
    });

    test('type alias declarations (function types)', () => {
      expectParseResult({
        source: `
          type alias Predicate = Number -> Boolean
          type alias Predicate2 = Number -> String -> Boolean
          type alias Predicate3 = (Number -> String) -> Boolean
        `,
        lineStart: 2,
        lineEnd: 4,
        body: [
          {
            type: nodeTypes.TYPE_ALIAS_DECLARATION,
            lineStart: 2,
            lineEnd: 2,
            typeName: buildNode.Identifier(2, 2)({
              value: 'Predicate',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: {
              type: nodeTypes.TYPE_FUNCTION,
              lineStart: 2,
              lineEnd: 2,
              from: {
                type: nodeTypes.TYPE_TAG,
                lineStart: 2,
                lineEnd: 2,
                typeTagName: buildNode.Identifier(2, 2)({
                  value: 'Number',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              },
              to: {
                type: nodeTypes.TYPE_TAG,
                lineStart: 2,
                lineEnd: 2,
                typeTagName: buildNode.Identifier(2, 2)({
                  value: 'Boolean',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              }
            }
          },
          {
            type: nodeTypes.TYPE_ALIAS_DECLARATION,
            lineStart: 3,
            lineEnd: 3,
            typeName: buildNode.Identifier(3, 3)({
              value: 'Predicate2',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: {
              type: nodeTypes.TYPE_FUNCTION,
              lineStart: 3,
              lineEnd: 3,
              from: {
                type: nodeTypes.TYPE_TAG,
                lineStart: 3,
                lineEnd: 3,
                typeTagName: buildNode.Identifier(3, 3)({
                  value: 'Number',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              },
              to: {
                type: nodeTypes.TYPE_FUNCTION,
                lineStart: 3,
                lineEnd: 3,
                from: {
                  type: nodeTypes.TYPE_TAG,
                  lineStart: 3,
                  lineEnd: 3,
                  typeTagName: buildNode.Identifier(3, 3)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                },
                to: {
                  type: nodeTypes.TYPE_TAG,
                  lineStart: 3,
                  lineEnd: 3,
                  typeTagName: buildNode.Identifier(3, 3)({
                    value: 'Boolean',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                }
              }
            }
          },
          {
            type: nodeTypes.TYPE_ALIAS_DECLARATION,
            lineStart: 4,
            lineEnd: 4,
            typeName: buildNode.Identifier(4, 4)({
              value: 'Predicate3',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: {
              type: nodeTypes.TYPE_FUNCTION,
              lineStart: 4,
              lineEnd: 4,
              from: {
                type: nodeTypes.TYPE_FUNCTION,
                lineStart: 4,
                lineEnd: 4,
                from: {
                  type: nodeTypes.TYPE_TAG,
                  lineStart: 4,
                  lineEnd: 4,
                  typeTagName: buildNode.Identifier(4, 4)({
                    value: 'Number',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                },
                to: {
                  type: nodeTypes.TYPE_TAG,
                  lineStart: 4,
                  lineEnd: 4,
                  typeTagName: buildNode.Identifier(4, 4)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                }
              },
              to: {
                type: nodeTypes.TYPE_TAG,
                lineStart: 4,
                lineEnd: 4,
                typeTagName: buildNode.Identifier(4, 4)({
                  value: 'Boolean',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              }
            }
          }
        ]
      });
    });

    xtest('type alias declarations (tuple types)', () => {
      expectParseResult({
        source: `
          type alias StringPair = (String, String)
          #type alias FunkyPair = (String, String -> Boolean)
        `,
        lineStart: 2,
        lineEnd: 3,
        body: []
      });
    });

    xtest('type alias declarations (record types)', () => {
      expectParseResult({
        source: `
          type alias Person = { name :: String, age :: Number }
        `,
        lineStart: 1,
        lineEnd: 2,
        body: []
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
            comments: [],
            lineStart: 2,
            lineEnd: 2,
            leftSide: buildNode.Identifier(2, 2)({
              value: 'func1',
              isGetter: false,
              isSetter: false
            }),
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
                  callee: buildNode.Identifier(2, 2)({
                    value: 'helloWorld',
                    isGetter: false,
                    isSetter: false
                  }),
                  arg: buildNode.Number(2, 2)({ value: 47 })
                },
                arg: {
                  type: nodeTypes.STRING,
                  value: 'something here',
                  lineStart: 2,
                  lineEnd: 2
                }
              },
              arg: buildNode.Identifier(2, 2)({
                value: 'cool',
                isGetter: false,
                isSetter: false
              })
            }
          },
          {
            type: nodeTypes.ASSIGNMENT,
            comments: [],
            lineStart: 3,
            lineEnd: 3,
            leftSide: buildNode.Identifier(3, 3)({
              value: 'func2',
              isGetter: false,
              isSetter: false
            }),
            rightSide: {
              type: nodeTypes.CALL,
              lineStart: 3,
              lineEnd: 3,
              callee: buildNode.Identifier(3, 3)({
                value: 'func1',
                isGetter: false,
                isSetter: false
              }),
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
            comments: [],
            lineStart: 2,
            lineEnd: 2,
            leftSide: buildNode.Identifier(2, 2)({
              value: 'fn',
              isGetter: false,
              isSetter: false
            }),
            rightSide: {
              type: nodeTypes.FUNCTION,
              lineStart: 2,
              lineEnd: 2,
              parameter: buildNode.Identifier(2, 2)({
                value: 'a',
                isGetter: false,
                isSetter: false
              }),
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
            callee: buildNode.Identifier(4, 4)({
              value: 'fn',
              isGetter: false,
              isSetter: false
            }),
            arg: buildNode.Number(4, 4)({ value: 1 })
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
            comments: [],
            lineStart: 2,
            lineEnd: 2,
            leftSide: buildNode.Identifier(2, 2)({
              value: 'toStr',
              isGetter: false,
              isSetter: false
            }),
            rightSide: {
              type: nodeTypes.FUNCTION,
              lineStart: 2,
              lineEnd: 2,
              parameter: buildNode.Identifier(2, 2)({
                value: 's',
                isGetter: false,
                isSetter: false
              }),
              body: {
                type: nodeTypes.CALL,
                lineStart: 2,
                lineEnd: 2,
                callee: buildNode.Identifier(2, 2)({
                  value: 'fn',
                  isGetter: false,
                  isSetter: false
                }),
                arg: buildNode.Identifier(2, 2)({
                  value: 's',
                  isGetter: false,
                  isSetter: false
                })
              }
            }
          },
          {
            type: nodeTypes.ARRAY,
            lineStart: 4,
            lineEnd: 4,
            elements: [
              buildNode.Number(4, 4)({ value: 1 }),
              buildNode.Number(4, 4)({ value: 2 }),
              buildNode.Number(4, 4)({ value: 3 })
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
        'Expected closing ")" to match opening "(" at line 1, column 0, but found'
      );
    });

    test('unclosed array brackets', () => {
      expectParseError(
        'fn [1, 2, 3',
        'Expected closing "]" to match opening "[" at line 1, column 3, but found'
      );
    });
  });
});
