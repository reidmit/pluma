import parse from '../src/parser';
import tokenize from '../src/tokenizer';
import { buildNode } from '../src/ast-nodes';

const expectParseResult = ({
  source,
  lineStart,
  lineEnd,
  interop = false,
  name = null,
  comments = [],
  exports = [],
  imports = [],
  body
}) => {
  const tokens = tokenize({ source });
  expect(parse({ source, tokens })).toEqual(
    buildNode.Module(lineStart, lineEnd)({
      interop,
      name,
      comments,
      exports,
      imports,
      body
    })
  );
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
          buildNode.MemberExpression(1, 1)({
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
          })
        ]
      });
    });

    test('function (one param)', () => {
      expectParseResult({
        source: 'x => 47',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Function(1, 1)({
            parameter: buildNode.Identifier(1, 1)({
              value: 'x',
              isGetter: false,
              isSetter: false
            }),
            body: buildNode.Number(1, 1)({ value: 47 })
          })
        ]
      });
    });

    test('function (two params)', () => {
      expectParseResult({
        source: 'x => y => 47',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Function(1, 1)({
            parameter: buildNode.Identifier(1, 1)({
              value: 'x',
              isGetter: false,
              isSetter: false
            }),
            body: buildNode.Function(1, 1)({
              parameter: buildNode.Identifier(1, 1)({
                value: 'y',
                isGetter: false,
                isSetter: false
              }),
              body: buildNode.Number(1, 1)({ value: 47 })
            })
          })
        ]
      });
    });

    test('assignment (without type annotation)', () => {
      expectParseResult({
        source: `
          hello = 47
          someString = "hello, world!"
        `,
        lineStart: 2,
        lineEnd: 3,
        body: [
          buildNode.Assignment(2, 2)({
            comments: [],
            id: buildNode.Identifier(2, 2)({
              value: 'hello',
              isGetter: false,
              isSetter: false
            }),
            typeAnnotation: null,
            value: buildNode.Number(2, 2)({ value: 47 })
          }),
          buildNode.Assignment(3, 3)({
            comments: [],
            id: buildNode.Identifier(3, 3)({
              value: 'someString',
              isGetter: false,
              isSetter: false
            }),
            typeAnnotation: null,
            value: buildNode.String(3, 3)({
              value: 'hello, world!'
            })
          })
        ]
      });
    });

    test('assignment (with type annotation)', () => {
      expectParseResult({
        source: `
          hello :: Number = 47
          someString :: String = "hello, world!"
          f2 :: String -> Number -> Boolean
            = s => n => False
        `,
        lineStart: 2,
        lineEnd: 5,
        body: [
          buildNode.Assignment(2, 2)({
            comments: [],
            id: buildNode.Identifier(2, 2)({
              value: 'hello',
              isGetter: false,
              isSetter: false
            }),
            typeAnnotation: buildNode.TypeTag(2, 2)({
              typeExpression: null,
              typeTagName: buildNode.Identifier(2, 2)({
                value: 'Number',
                isGetter: false,
                isSetter: false
              })
            }),
            value: buildNode.Number(2, 2)({ value: 47 })
          }),
          buildNode.Assignment(3, 3)({
            comments: [],
            id: buildNode.Identifier(3, 3)({
              value: 'someString',
              isGetter: false,
              isSetter: false
            }),
            typeAnnotation: buildNode.TypeTag(3, 3)({
              typeExpression: null,
              typeTagName: buildNode.Identifier(3, 3)({
                value: 'String',
                isGetter: false,
                isSetter: false
              })
            }),
            value: buildNode.String(3, 3)({
              value: 'hello, world!'
            })
          }),
          buildNode.Assignment(4, 5)({
            comments: [],
            id: buildNode.Identifier(4, 4)({
              value: 'f2',
              isGetter: false,
              isSetter: false
            }),
            typeAnnotation: buildNode.FunctionType(4, 4)({
              from: buildNode.TypeTag(4, 4)({
                typeExpression: null,
                typeTagName: buildNode.Identifier(4, 4)({
                  value: 'String',
                  isGetter: false,
                  isSetter: false
                })
              }),
              to: buildNode.FunctionType(4, 4)({
                from: buildNode.TypeTag(4, 4)({
                  typeExpression: null,
                  typeTagName: buildNode.Identifier(4, 4)({
                    value: 'Number',
                    isGetter: false,
                    isSetter: false
                  })
                }),
                to: buildNode.TypeTag(4, 4)({
                  typeExpression: null,
                  typeTagName: buildNode.Identifier(4, 4)({
                    value: 'Boolean',
                    isGetter: false,
                    isSetter: false
                  })
                })
              })
            }),
            value: buildNode.Function(5, 5)({
              parameter: buildNode.Identifier(5, 5)({
                value: 's',
                isGetter: false,
                isSetter: false
              }),
              body: buildNode.Function(5, 5)({
                parameter: buildNode.Identifier(5, 5)({
                  value: 'n',
                  isGetter: false,
                  isSetter: false
                }),
                body: buildNode.Boolean(5, 5)({
                  value: false
                })
              })
            })
          })
        ]
      });
    });

    test('call expression (getter function)', () => {
      expectParseResult({
        source: '.someProp someObject',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Call(1, 1)({
            callee: buildNode.Identifier(1, 1)({
              value: 'someProp',
              isGetter: true,
              isSetter: false
            }),
            argument: buildNode.Identifier(1, 1)({
              value: 'someObject',
              isGetter: false,
              isSetter: false
            })
          })
        ]
      });
    });

    test('call expression (setter function)', () => {
      expectParseResult({
        source: '@someProp 47 someObject',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Call(1, 1)({
            callee: buildNode.Call(1, 1)({
              callee: buildNode.Identifier(1, 1)({
                value: 'someProp',
                isGetter: false,
                isSetter: true
              }),
              argument: buildNode.Number(1, 1)({ value: 47 })
            }),
            argument: buildNode.Identifier(1, 1)({
              value: 'someObject',
              isGetter: false,
              isSetter: false
            })
          })
        ]
      });
    });

    test('call expression (single argument)', () => {
      expectParseResult({
        source: 'someFunc someArg',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Call(1, 1)({
            callee: buildNode.Identifier(1, 1)({
              value: 'someFunc',
              isGetter: false,
              isSetter: false
            }),
            argument: buildNode.Identifier(1, 1)({
              value: 'someArg',
              isGetter: false,
              isSetter: false
            })
          })
        ]
      });
    });

    test('call expression (multiple arguments)', () => {
      expectParseResult({
        source: 'helloWorld 47 "something here" cool',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Call(1, 1)({
            callee: buildNode.Call(1, 1)({
              callee: buildNode.Call(1, 1)({
                callee: buildNode.Identifier(1, 1)({
                  value: 'helloWorld',
                  isGetter: false,
                  isSetter: false
                }),
                argument: buildNode.Number(1, 1)({ value: 47 })
              }),
              argument: buildNode.String(1, 1)({
                value: 'something here'
              })
            }),
            argument: buildNode.Identifier(1, 1)({
              value: 'cool',
              isGetter: false,
              isSetter: false
            })
          })
        ]
      });
    });

    test('call expression (multiple identifier arguments)', () => {
      expectParseResult({
        source: 'combine a b c',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Call(1, 1)({
            callee: buildNode.Call(1, 1)({
              callee: buildNode.Call(1, 1)({
                callee: buildNode.Identifier(1, 1)({
                  value: 'combine',
                  isGetter: false,
                  isSetter: false
                }),
                argument: buildNode.Identifier(1, 1)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                })
              }),
              argument: buildNode.Identifier(1, 1)({
                value: 'b',
                isGetter: false,
                isSetter: false
              })
            }),
            argument: buildNode.Identifier(1, 1)({
              value: 'c',
              isGetter: false,
              isSetter: false
            })
          })
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
          buildNode.Call(2, 2)({
            callee: buildNode.Call(2, 2)({
              callee: buildNode.Identifier(2, 2)({
                value: 'someFunc',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Call(2, 2)({
                callee: buildNode.Identifier(2, 2)({
                  value: 'someOtherFunc',
                  isGetter: false,
                  isSetter: false
                }),
                argument: buildNode.Number(2, 2)({ value: 3 })
              })
            }),
            argument: buildNode.Number(2, 2)({ value: 4 })
          })
        ]
      });
    });

    test('array expressions (empty)', () => {
      expectParseResult({
        source: '[]',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Array(1, 1)({
            elements: []
          })
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
          buildNode.Array(1, 3)({
            elements: [
              buildNode.Number(2, 2)({ value: 1 }),
              buildNode.Identifier(2, 2)({
                value: 'test',
                isGetter: false,
                isSetter: false
              }),
              buildNode.Boolean(2, 2)({
                value: true
              }),
              buildNode.String(2, 2)({
                value: 'hello'
              })
            ]
          })
        ]
      });
    });

    test('record expressions (empty)', () => {
      expectParseResult({
        source: '{}',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Record(1, 1)({
            properties: []
          })
        ]
      });
    });

    test('record expressions (basic)', () => {
      expectParseResult({
        source: '{a: 1, b: "hello", c-d: True}',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Record(1, 1)({
            properties: [
              buildNode.RecordProperty(1, 1)({
                key: buildNode.Identifier(1, 1)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                }),
                value: buildNode.Number(1, 1)({ value: 1 })
              }),
              buildNode.RecordProperty(1, 1)({
                key: buildNode.Identifier(1, 1)({
                  value: 'b',
                  isGetter: false,
                  isSetter: false
                }),
                value: buildNode.String(1, 1)({
                  value: 'hello'
                })
              }),
              buildNode.RecordProperty(1, 1)({
                key: buildNode.Identifier(1, 1)({
                  value: 'c-d',
                  isGetter: false,
                  isSetter: false
                }),
                value: buildNode.Boolean(1, 1)({
                  value: true
                })
              })
            ]
          })
        ]
      });
    });

    test('record expressions (shorthand keys)', () => {
      expectParseResult({
        source: '{ short, hand }',
        lineStart: 1,
        lineEnd: 1,
        body: [
          buildNode.Record(1, 1)({
            properties: [
              buildNode.RecordProperty(1, 1)({
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
              }),
              buildNode.RecordProperty(1, 1)({
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
              })
            ]
          })
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
          buildNode.Tuple(2, 2)({
            entries: [
              buildNode.Number(2, 2)({ value: 1 }),
              buildNode.Boolean(2, 2)({
                value: true
              }),
              buildNode.String(2, 2)({
                value: 'hello'
              }),
              buildNode.Identifier(2, 2)({
                value: 'nice',
                isGetter: false,
                isSetter: false
              })
            ]
          }),
          buildNode.Tuple(3, 4)({
            entries: [
              buildNode.Number(3, 3)({ value: 3 }),
              buildNode.Number(3, 3)({ value: 4 }),
              buildNode.Number(4, 4)({ value: 5 })
            ]
          })
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
          buildNode.Conditional(2, 4)({
            predicate: buildNode.Boolean(2, 2)({
              value: true
            }),
            thenCase: buildNode.Number(3, 3)({ value: 47 }),
            elseCase: buildNode.Number(4, 4)({ value: 100 })
          }),
          buildNode.Conditional(6, 6)({
            predicate: buildNode.Call(6, 6)({
              callee: buildNode.Call(6, 6)({
                callee: buildNode.Identifier(6, 6)({
                  value: 'and',
                  isGetter: false,
                  isSetter: false
                }),
                argument: buildNode.Boolean(6, 6)({
                  value: true
                })
              }),
              argument: buildNode.Boolean(6, 6)({
                value: false
              })
            }),
            thenCase: buildNode.String(6, 6)({
              value: 'no'
            }),
            elseCase: buildNode.String(6, 6)({
              value: 'yes'
            })
          }),
          buildNode.Conditional(8, 10)({
            predicate: buildNode.Boolean(8, 8)({
              value: false
            }),
            thenCase: buildNode.String(8, 8)({
              value: 'okay'
            }),
            elseCase: buildNode.Conditional(9, 10)({
              predicate: buildNode.Boolean(9, 9)({
                value: true
              }),
              thenCase: buildNode.String(9, 9)({
                value: 'maybe'
              }),
              elseCase: buildNode.String(10, 10)({
                value: 'nah'
              })
            })
          })
        ]
      });
    });

    test('pipe expression (|>)', () => {
      expectParseResult({
        source: `
          "hello" |> length
          # separator comment
          47 |> gt 10
          # test
          id 47 |> gt 10
        `,
        lineStart: 2,
        lineEnd: 6,
        body: [
          buildNode.PipeExpression(2, 2)({
            left: buildNode.String(2, 2)({
              value: 'hello'
            }),
            right: buildNode.Identifier(2, 2)({
              value: 'length',
              isGetter: false,
              isSetter: false
            })
          }),
          buildNode.PipeExpression(4, 4)({
            left: buildNode.Number(4, 4)({
              value: 47
            }),
            right: buildNode.Call(4, 4)({
              callee: buildNode.Identifier(4, 4)({
                value: 'gt',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Number(4, 4)({
                value: 10
              })
            })
          }),
          buildNode.PipeExpression(6, 6)({
            left: buildNode.Call(6, 6)({
              callee: buildNode.Identifier(6, 6)({
                value: 'id',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Number(6, 6)({
                value: 47
              })
            }),
            right: buildNode.Call(6, 6)({
              callee: buildNode.Identifier(6, 6)({
                value: 'gt',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Number(6, 6)({
                value: 10
              })
            })
          })
        ]
      });
    });

    test('pipe expression (multiple pipes)', () => {
      expectParseResult({
        source: `
          "reid" |> p1 |> p2 |> greet
          "reid" |> p3 arg |> p4 |> greet2 arg
        `,
        lineStart: 2,
        lineEnd: 3,
        body: [
          buildNode.PipeExpression(2, 2)({
            left: buildNode.PipeExpression(2, 2)({
              left: buildNode.PipeExpression(2, 2)({
                left: buildNode.String(2, 2)({
                  value: 'reid'
                }),
                right: buildNode.Identifier(2, 2)({
                  value: 'p1',
                  isGetter: false,
                  isSetter: false
                })
              }),
              right: buildNode.Identifier(2, 2)({
                value: 'p2',
                isGetter: false,
                isSetter: false
              })
            }),
            right: buildNode.Identifier(2, 2)({
              value: 'greet',
              isGetter: false,
              isSetter: false
            })
          }),
          buildNode.PipeExpression(3, 3)({
            left: buildNode.PipeExpression(3, 3)({
              left: buildNode.PipeExpression(3, 3)({
                left: buildNode.String(3, 3)({
                  value: 'reid'
                }),
                right: buildNode.Call(3, 3)({
                  callee: buildNode.Identifier(3, 3)({
                    value: 'p3',
                    isGetter: false,
                    isSetter: false
                  }),
                  argument: buildNode.Identifier(3, 3)({
                    value: 'arg',
                    isGetter: false,
                    isSetter: false
                  })
                })
              }),
              right: buildNode.Identifier(3, 3)({
                value: 'p4',
                isGetter: false,
                isSetter: false
              })
            }),
            right: buildNode.Call(3, 3)({
              callee: buildNode.Identifier(3, 3)({
                value: 'greet2',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Identifier(3, 3)({
                value: 'arg',
                isGetter: false,
                isSetter: false
              })
            })
          })
        ]
      });
    });

    test('let-in expressions (simple)', () => {
      expectParseResult({
        source: `
          let a = 47 in add a
        `,
        lineStart: 2,
        lineEnd: 2,
        body: [
          buildNode.LetExpression(2, 2)({
            assignments: [
              buildNode.Assignment(2, 2)({
                comments: [],
                id: buildNode.Identifier(2, 2)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                }),
                typeAnnotation: null,
                value: buildNode.Number(2, 2)({ value: 47 })
              })
            ],
            body: buildNode.Call(2, 2)({
              callee: buildNode.Identifier(2, 2)({
                value: 'add',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Identifier(2, 2)({
                value: 'a',
                isGetter: false,
                isSetter: false
              })
            })
          })
        ]
      });
    });

    test('let-in expressions (multiple assignments)', () => {
      expectParseResult({
        source: `
          let
            a = 47
            b = "hello"
          in
            combine a b
        `,
        lineStart: 2,
        lineEnd: 6,
        body: [
          buildNode.LetExpression(2, 6)({
            assignments: [
              buildNode.Assignment(3, 3)({
                comments: [],
                id: buildNode.Identifier(3, 3)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                }),
                typeAnnotation: null,
                value: buildNode.Number(3, 3)({ value: 47 })
              }),
              buildNode.Assignment(4, 4)({
                comments: [],
                id: buildNode.Identifier(4, 4)({
                  value: 'b',
                  isGetter: false,
                  isSetter: false
                }),
                typeAnnotation: null,
                value: buildNode.String(4, 4)({ value: 'hello' })
              })
            ],
            body: buildNode.Call(6, 6)({
              callee: buildNode.Call(6, 6)({
                callee: buildNode.Identifier(6, 6)({
                  value: 'combine',
                  isGetter: false,
                  isSetter: false
                }),
                argument: buildNode.Identifier(6, 6)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                })
              }),
              argument: buildNode.Identifier(6, 6)({
                value: 'b',
                isGetter: false,
                isSetter: false
              })
            })
          })
        ]
      });
    });

    test('comments', () => {
      expectParseResult({
        source: `
          # This is a comment that
          # should be preserved for the below assignment
          x = 47 # but not this

          # or this
        `,
        lineStart: 2,
        lineEnd: 4,
        body: [
          buildNode.Assignment(2, 4)({
            comments: [
              ' This is a comment that',
              ' should be preserved for the below assignment'
            ],
            id: buildNode.Identifier(4, 4)({
              value: 'x',
              isGetter: false,
              isSetter: false
            }),
            typeAnnotation: null,
            value: buildNode.Number(4, 4)({ value: 47 })
          })
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
          buildNode.TypeDeclaration(2, 2)({
            comments: [],
            typeName: buildNode.Identifier(2, 2)({
              value: 'Letter',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeConstructors: [
              buildNode.TypeConstructor(2, 2)({
                typeName: buildNode.Identifier(2, 2)({
                  value: 'Alpha',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              }),
              buildNode.TypeConstructor(2, 2)({
                typeName: buildNode.Identifier(2, 2)({
                  value: 'Beta',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              }),
              buildNode.TypeConstructor(2, 2)({
                typeName: buildNode.Identifier(2, 2)({
                  value: 'Gamma',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              })
            ]
          }),
          buildNode.TypeDeclaration(3, 5)({
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
              buildNode.TypeConstructor(4, 4)({
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
              }),
              buildNode.TypeConstructor(5, 5)({
                typeName: buildNode.Identifier(5, 5)({
                  value: 'Nothing',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              })
            ]
          }),
          buildNode.TypeDeclaration(7, 8)({
            comments: [' Type declarations can have comments'],
            typeName: buildNode.Identifier(8, 8)({
              value: 'Hello',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeConstructors: [
              buildNode.TypeConstructor(8, 8)({
                typeName: buildNode.Identifier(8, 8)({
                  value: 'World',
                  isGetter: false,
                  isSetter: false
                }),
                typeParameters: []
              })
            ]
          })
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
          buildNode.TypeAliasDeclaration(2, 2)({
            typeName: buildNode.Identifier(2, 2)({
              value: 'Hello',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.TypeTag(2, 2)({
              typeTagName: buildNode.Identifier(2, 2)({
                value: 'String',
                isGetter: false,
                isSetter: false
              }),
              typeExpression: null
            })
          }),
          buildNode.TypeAliasDeclaration(3, 3)({
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
            typeExpression: buildNode.TypeTag(3, 3)({
              typeTagName: buildNode.Identifier(3, 3)({
                value: 'Something',
                isGetter: false,
                isSetter: false
              }),
              typeExpression: buildNode.TypeVariable(3, 3)({
                typeName: buildNode.Identifier(3, 3)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                })
              })
            })
          })
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
          buildNode.TypeAliasDeclaration(2, 2)({
            typeName: buildNode.Identifier(2, 2)({
              value: 'Predicate',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.FunctionType(2, 2)({
              from: buildNode.TypeTag(2, 2)({
                typeTagName: buildNode.Identifier(2, 2)({
                  value: 'Number',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              }),
              to: buildNode.TypeTag(2, 2)({
                typeTagName: buildNode.Identifier(2, 2)({
                  value: 'Boolean',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              })
            })
          }),
          buildNode.TypeAliasDeclaration(3, 3)({
            typeName: buildNode.Identifier(3, 3)({
              value: 'Predicate2',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.FunctionType(3, 3)({
              from: buildNode.TypeTag(3, 3)({
                typeTagName: buildNode.Identifier(3, 3)({
                  value: 'Number',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              }),
              to: buildNode.FunctionType(3, 3)({
                from: buildNode.TypeTag(3, 3)({
                  typeTagName: buildNode.Identifier(3, 3)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                }),
                to: buildNode.TypeTag(3, 3)({
                  typeTagName: buildNode.Identifier(3, 3)({
                    value: 'Boolean',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                })
              })
            })
          }),
          buildNode.TypeAliasDeclaration(4, 4)({
            typeName: buildNode.Identifier(4, 4)({
              value: 'Predicate3',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.FunctionType(4, 4)({
              from: buildNode.FunctionType(4, 4)({
                from: buildNode.TypeTag(4, 4)({
                  typeTagName: buildNode.Identifier(4, 4)({
                    value: 'Number',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                }),
                to: buildNode.TypeTag(4, 4)({
                  typeTagName: buildNode.Identifier(4, 4)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                })
              }),
              to: buildNode.TypeTag(4, 4)({
                typeTagName: buildNode.Identifier(4, 4)({
                  value: 'Boolean',
                  isGetter: false,
                  isSetter: false
                }),
                typeExpression: null
              })
            })
          })
        ]
      });
    });

    test('type alias declarations (tuple types)', () => {
      expectParseResult({
        source: `
          type alias StringPair = (String, String)
          type alias FunkyPair = (String, String -> Boolean)
        `,
        lineStart: 2,
        lineEnd: 3,
        body: [
          buildNode.TypeAliasDeclaration(2, 2)({
            typeName: buildNode.Identifier(2, 2)({
              value: 'StringPair',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.TupleType(2, 2)({
              typeEntries: [
                buildNode.TypeTag(2, 2)({
                  typeTagName: buildNode.Identifier(2, 2)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                }),
                buildNode.TypeTag(2, 2)({
                  typeTagName: buildNode.Identifier(2, 2)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                })
              ]
            })
          }),
          buildNode.TypeAliasDeclaration(3, 3)({
            typeName: buildNode.Identifier(3, 3)({
              value: 'FunkyPair',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.TupleType(3, 3)({
              typeEntries: [
                buildNode.TypeTag(3, 3)({
                  typeTagName: buildNode.Identifier(3, 3)({
                    value: 'String',
                    isGetter: false,
                    isSetter: false
                  }),
                  typeExpression: null
                }),
                buildNode.FunctionType(3, 3)({
                  from: buildNode.TypeTag(3, 3)({
                    typeTagName: buildNode.Identifier(3, 3)({
                      value: 'String',
                      isGetter: false,
                      isSetter: false
                    }),
                    typeExpression: null
                  }),
                  to: buildNode.TypeTag(3, 3)({
                    typeTagName: buildNode.Identifier(3, 3)({
                      value: 'Boolean',
                      isGetter: false,
                      isSetter: false
                    }),
                    typeExpression: null
                  })
                })
              ]
            })
          })
        ]
      });
    });

    test('type alias declarations (record types)', () => {
      expectParseResult({
        source: `
          type alias Person = { name :: String, age :: Number, test :: String -> Boolean }
        `,
        lineStart: 2,
        lineEnd: 2,
        body: [
          buildNode.TypeAliasDeclaration(2, 2)({
            typeName: buildNode.Identifier(2, 2)({
              value: 'Person',
              isGetter: false,
              isSetter: false
            }),
            typeParameters: [],
            typeExpression: buildNode.RecordType(2, 2)({
              properties: [
                buildNode.RecordPropertyType(2, 2)({
                  key: buildNode.Identifier(2, 2)({
                    value: 'name',
                    isGetter: false,
                    isSetter: false
                  }),
                  value: buildNode.TypeTag(2, 2)({
                    typeTagName: buildNode.Identifier(2, 2)({
                      value: 'String',
                      isGetter: false,
                      isSetter: false
                    }),
                    typeExpression: null
                  })
                }),
                buildNode.RecordPropertyType(2, 2)({
                  key: buildNode.Identifier(2, 2)({
                    value: 'age',
                    isGetter: false,
                    isSetter: false
                  }),
                  value: buildNode.TypeTag(2, 2)({
                    typeTagName: buildNode.Identifier(2, 2)({
                      value: 'Number',
                      isGetter: false,
                      isSetter: false
                    }),
                    typeExpression: null
                  })
                }),
                buildNode.RecordPropertyType(2, 2)({
                  key: buildNode.Identifier(2, 2)({
                    value: 'test',
                    isGetter: false,
                    isSetter: false
                  }),
                  value: buildNode.FunctionType(2, 2)({
                    from: buildNode.TypeTag(2, 2)({
                      typeTagName: buildNode.Identifier(2, 2)({
                        value: 'String',
                        isGetter: false,
                        isSetter: false
                      }),
                      typeExpression: null
                    }),
                    to: buildNode.TypeTag(2, 2)({
                      typeTagName: buildNode.Identifier(2, 2)({
                        value: 'Boolean',
                        isGetter: false,
                        isSetter: false
                      }),
                      typeExpression: null
                    })
                  })
                })
              ]
            })
          })
        ]
      });
    });

    test('module declaration (no comments)', () => {
      expectParseResult({
        source: `
          module SomeModule
        `,
        lineStart: 2,
        lineEnd: 2,
        name: buildNode.Identifier(2, 2)({
          value: 'SomeModule',
          isGetter: false,
          isSetter: false
        }),
        comments: [],
        body: []
      });
    });

    test('module declaration (with comments)', () => {
      expectParseResult({
        source: `
          # This is a test module
          # that has nothing in it
          module SomeModule
        `,
        lineStart: 2,
        lineEnd: 4,
        name: buildNode.Identifier(4, 4)({
          value: 'SomeModule',
          isGetter: false,
          isSetter: false
        }),
        comments: [' This is a test module', ' that has nothing in it'],
        body: []
      });
    });

    test('interop module declaration', () => {
      expectParseResult({
        source: `
          interop module SomeModule
        `,
        lineStart: 2,
        lineEnd: 2,
        name: buildNode.Identifier(2, 2)({
          value: 'SomeModule',
          isGetter: false,
          isSetter: false
        }),
        interop: true,
        comments: [],
        body: []
      });
    });

    test('export statement', () => {
      expectParseResult({
        source: `
          export (func1, nice)
        `,
        lineStart: 2,
        lineEnd: 2,
        exports: [
          buildNode.Identifier(2, 2)({
            value: 'func1',
            isGetter: false,
            isSetter: false
          }),
          buildNode.Identifier(2, 2)({
            value: 'nice',
            isGetter: false,
            isSetter: false
          })
        ],
        body: []
      });
    });

    test('import statements', () => {
      expectParseResult({
        source: `
          import (func1, nice) from Some.OtherModule
          import AnotherModule
        `,
        lineStart: 2,
        lineEnd: 3,
        imports: [
          buildNode.Import(2, 2)({
            identifiers: [
              buildNode.Identifier(2, 2)({
                value: 'func1',
                isGetter: false,
                isSetter: false
              }),
              buildNode.Identifier(2, 2)({
                value: 'nice',
                isGetter: false,
                isSetter: false
              })
            ],
            module: buildNode.MemberExpression(2, 2)({
              parts: [
                buildNode.Identifier(2, 2)({
                  value: 'Some',
                  isGetter: false,
                  isSetter: false
                }),
                buildNode.Identifier(2, 2)({
                  value: 'OtherModule',
                  isGetter: false,
                  isSetter: false
                })
              ]
            })
          }),
          buildNode.Import(3, 3)({
            identifiers: null,
            module: buildNode.Identifier(3, 3)({
              value: 'AnotherModule',
              isGetter: false,
              isSetter: false
            })
          })
        ],
        body: []
      });
    });

    describe('edge cases', () => {
      test('multiple complex assignments', () => {
        expectParseResult({
          source: `
          func1 = helloWorld 47 "something here" cool
          func2 = func1 True
        `,
          lineStart: 2,
          lineEnd: 3,
          body: [
            buildNode.Assignment(2, 2)({
              comments: [],
              id: buildNode.Identifier(2, 2)({
                value: 'func1',
                isGetter: false,
                isSetter: false
              }),
              typeAnnotation: null,
              value: buildNode.Call(2, 2)({
                callee: buildNode.Call(2, 2)({
                  callee: buildNode.Call(2, 2)({
                    callee: buildNode.Identifier(2, 2)({
                      value: 'helloWorld',
                      isGetter: false,
                      isSetter: false
                    }),
                    argument: buildNode.Number(2, 2)({ value: 47 })
                  }),
                  argument: buildNode.String(2, 2)({
                    value: 'something here'
                  })
                }),
                argument: buildNode.Identifier(2, 2)({
                  value: 'cool',
                  isGetter: false,
                  isSetter: false
                })
              })
            }),
            buildNode.Assignment(3, 3)({
              comments: [],
              id: buildNode.Identifier(3, 3)({
                value: 'func2',
                isGetter: false,
                isSetter: false
              }),
              typeAnnotation: null,
              value: buildNode.Call(3, 3)({
                callee: buildNode.Identifier(3, 3)({
                  value: 'func1',
                  isGetter: false,
                  isSetter: false
                }),
                argument: buildNode.Boolean(3, 3)({
                  value: true
                })
              })
            })
          ]
        });
      });

      test('function definition followed by call', () => {
        expectParseResult({
          source: `
          fn = a => "hello, world!"

          fn 1
        `,
          lineStart: 2,
          lineEnd: 4,
          body: [
            buildNode.Assignment(2, 2)({
              comments: [],
              id: buildNode.Identifier(2, 2)({
                value: 'fn',
                isGetter: false,
                isSetter: false
              }),
              typeAnnotation: null,
              value: buildNode.Function(2, 2)({
                parameter: buildNode.Identifier(2, 2)({
                  value: 'a',
                  isGetter: false,
                  isSetter: false
                }),
                body: buildNode.String(2, 2)({
                  value: 'hello, world!'
                })
              })
            }),
            buildNode.Call(4, 4)({
              callee: buildNode.Identifier(4, 4)({
                value: 'fn',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Number(4, 4)({ value: 1 })
            })
          ]
        });
      });

      test('function definition followed by array', () => {
        expectParseResult({
          source: `
          toStr = s => fn s

          [1, 2, 3]
        `,
          lineStart: 2,
          lineEnd: 4,
          body: [
            buildNode.Assignment(2, 2)({
              comments: [],
              id: buildNode.Identifier(2, 2)({
                value: 'toStr',
                isGetter: false,
                isSetter: false
              }),
              typeAnnotation: null,
              value: buildNode.Function(2, 2)({
                parameter: buildNode.Identifier(2, 2)({
                  value: 's',
                  isGetter: false,
                  isSetter: false
                }),
                body: buildNode.Call(2, 2)({
                  callee: buildNode.Identifier(2, 2)({
                    value: 'fn',
                    isGetter: false,
                    isSetter: false
                  }),
                  argument: buildNode.Identifier(2, 2)({
                    value: 's',
                    isGetter: false,
                    isSetter: false
                  })
                })
              })
            }),
            buildNode.Array(4, 4)({
              elements: [
                buildNode.Number(4, 4)({ value: 1 }),
                buildNode.Number(4, 4)({ value: 2 }),
                buildNode.Number(4, 4)({ value: 3 })
              ]
            })
          ]
        });
      });

      test('let expression followed by function call', () => {
        expectParseResult({
          source: `module Test

withLetExpression = firstName =>
  let
    lastName = "test"
  in
    "hi!"

fn 1
`,
          lineStart: 1,
          lineEnd: 9,
          name: buildNode.Identifier(1, 1)({
            value: 'Test',
            isGetter: false,
            isSetter: false
          }),
          body: [
            buildNode.Assignment(3, 7)({
              id: buildNode.Identifier(3, 3)({
                value: 'withLetExpression',
                isGetter: false,
                isSetter: false
              }),
              typeAnnotation: null,
              comments: [],
              value: buildNode.Function(3, 7)({
                parameter: buildNode.Identifier(3, 3)({
                  value: 'firstName',
                  isGetter: false,
                  isSetter: false
                }),
                body: buildNode.LetExpression(4, 7)({
                  assignments: [
                    buildNode.Assignment(5, 5)({
                      id: buildNode.Identifier(5, 5)({
                        value: 'lastName',
                        isGetter: false,
                        isSetter: false
                      }),
                      typeAnnotation: null,
                      comments: [],
                      value: buildNode.String(5, 5)({
                        value: 'test'
                      })
                    })
                  ],
                  body: buildNode.String(7, 7)({
                    value: 'hi!'
                  })
                })
              })
            }),
            buildNode.Call(9, 9)({
              callee: buildNode.Identifier(9, 9)({
                value: 'fn',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Number(9, 9)({
                value: 1
              })
            })
          ]
        });
      });

      test('consecutive function calls with same level of indentation', () => {
        expectParseResult({
          source: `
            f one
            g two
          `,
          lineStart: 2,
          lineEnd: 3,
          body: [
            buildNode.Call(2, 2)({
              callee: buildNode.Identifier(2, 2)({
                value: 'f',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Identifier(2, 2)({
                value: 'one',
                isGetter: false,
                isSetter: false
              })
            }),
            buildNode.Call(3, 3)({
              callee: buildNode.Identifier(3, 3)({
                value: 'g',
                isGetter: false,
                isSetter: false
              }),
              argument: buildNode.Identifier(3, 3)({
                value: 'two',
                isGetter: false,
                isSetter: false
              })
            })
          ]
        });
      });

      test('file with only module name and import', () => {
        expectParseResult({
          source: `
          module CircularA
          import Circular.CircularB`,
          lineStart: 2,
          lineEnd: 3,
          name: buildNode.Identifier(2, 2)({
            value: 'CircularA',
            isGetter: false,
            isSetter: false
          }),
          imports: [
            buildNode.Import(3, 3)({
              identifiers: null,
              module: buildNode.MemberExpression(3, 3)({
                parts: [
                  buildNode.Identifier(3, 3)({
                    value: 'Circular',
                    isGetter: false,
                    isSetter: false
                  }),
                  buildNode.Identifier(3, 3)({
                    value: 'CircularB',
                    isGetter: false,
                    isSetter: false
                  })
                ]
              })
            })
          ],
          body: []
        });
      });
    });
  });

  describe('error cases', () => {
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
