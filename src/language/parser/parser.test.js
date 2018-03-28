import parse from './parser';
import tokenize from '../tokenizer';
import * as t from 'babel-types';

const expectAst = (source, bodyNodes) => {
  const tokens = tokenize({ source });
  expect(parse({ source, tokens })).toEqual(
    t.file(t.program(bodyNodes), [], tokens)
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
  expect(error.name).toBe('Parser error');
  expect(error.message).toMatch(errorMessageRegex);
};

describe('parser', () => {
  describe('valid programs', () => {
    test('empty program', () => {
      expectAst('', []);
    });

    test('null literal', () => {
      expectAst('null', [t.expressionStatement(t.nullLiteral())]);
    });

    test('number literal', () => {
      expectAst('47', [t.expressionStatement(t.numericLiteral(47))]);
    });

    test('boolean literal', () => {
      expectAst('True', [t.expressionStatement(t.booleanLiteral(true))]);
    });

    test('identifier', () => {
      expectAst('lol', [t.expressionStatement(t.identifier('lol'))]);
    });

    test('string literal', () => {
      expectAst('"hello, world!"', [
        t.expressionStatement(t.stringLiteral('hello, world!'))
      ]);
    });

    test('interpolated string literal', () => {
      expectAst('"hello, ${name}!"', [
        t.expressionStatement(
          t.templateLiteral(
            [
              t.templateElement({ raw: 'hello, ', cooked: 'hello, ' }, false),
              t.templateElement({ raw: '!', cooked: '!' }, true)
            ],
            [t.identifier('name')]
          )
        )
      ]);
    });

    test('member expression with dots (non-computed)', () => {
      expectAst('a.b.c', [
        t.expressionStatement(
          t.memberExpression(
            t.memberExpression(t.identifier('a'), t.identifier('b'), false),
            t.identifier('c'),
            false
          )
        )
      ]);
    });

    test('member expression with brackets (computed)', () => {
      expectAst('a[b]["hello"][0]', [
        t.expressionStatement(
          t.memberExpression(
            t.memberExpression(
              t.memberExpression(t.identifier('a'), t.identifier('b'), true),
              t.stringLiteral('hello'),
              true
            ),
            t.numericLiteral(0),
            true
          )
        )
      ]);
    });

    test('function (one param)', () => {
      expectAst('x => 47', [
        t.expressionStatement(
          t.arrowFunctionExpression([t.identifier('x')], t.numericLiteral(47))
        )
      ]);
    });

    test('function (async)', () => {
      expectAst('async x => 47', [
        t.expressionStatement(
          t.arrowFunctionExpression(
            [t.identifier('x')],
            t.numericLiteral(47),
            true
          )
        )
      ]);
    });

    test('function (two params)', () => {
      expectAst('x => y => 47', [
        t.expressionStatement(
          t.arrowFunctionExpression(
            [t.identifier('x')],
            t.arrowFunctionExpression(
              [t.identifier('y')],
              t.numericLiteral(47),
              false
            )
          )
        )
      ]);
    });

    test('assignment', () => {
      expectAst(
        `
      let hello = 47
      let someString = "hello, world!"
    `,
        [
          t.variableDeclaration('const', [
            t.variableDeclarator(t.identifier('hello'), t.numericLiteral(47))
          ]),
          t.variableDeclaration('const', [
            t.variableDeclarator(
              t.identifier('someString'),
              t.stringLiteral('hello, world!')
            )
          ])
        ]
      );
    });

    test('call expression (single argument)', () => {
      expectAst('someFunc someArg', [
        t.expressionStatement(
          t.callExpression(t.identifier('someFunc'), [t.identifier('someArg')])
        )
      ]);
    });

    test('call expression (multiple arguments)', () => {
      expectAst('helloWorld 47 "something here" cool', [
        t.expressionStatement(
          t.callExpression(
            t.callExpression(
              t.callExpression(t.identifier('helloWorld'), [
                t.numericLiteral(47)
              ]),
              [t.stringLiteral('something here')]
            ),
            [t.identifier('cool')]
          )
        )
      ]);
    });

    test('nested call expressions with parentheses', () => {
      expectAst(
        `
      someFunc (someOtherFunc 3) 4
    `,
        [
          t.expressionStatement(
            t.callExpression(
              t.callExpression(t.identifier('someFunc'), [
                t.callExpression(t.identifier('someOtherFunc'), [
                  t.numericLiteral(3)
                ])
              ]),
              [t.numericLiteral(4)]
            )
          )
        ]
      );
    });

    test('array expressions (empty)', () => {
      expectAst('[]', [t.expressionStatement(t.arrayExpression([]))]);
    });

    test('array expressions (basic)', () => {
      expectAst(
        `
        [1, test, True, "hello"]
      `,
        [
          t.expressionStatement(
            t.arrayExpression([
              t.numericLiteral(1),
              t.identifier('test'),
              t.booleanLiteral(true),
              t.stringLiteral('hello')
            ])
          )
        ]
      );
    });

    test('object expressions (empty)', () => {
      expectAst('{}', [t.expressionStatement(t.objectExpression([]))]);
    });

    test('object expressions (basic)', () => {
      expectAst('{a: 1, b: "hello", "c d": True}', [
        t.expressionStatement(
          t.objectExpression([
            t.objectProperty(t.identifier('a'), t.numericLiteral(1)),
            t.objectProperty(t.identifier('b'), t.stringLiteral('hello')),
            t.objectProperty(t.stringLiteral('c d'), t.booleanLiteral(true))
          ])
        )
      ]);
    });

    test('object expressions (computed keys)', () => {
      expectAst('{ [something]: 1, ["test"]: 2 }', [
        t.expressionStatement(
          t.objectExpression([
            t.objectProperty(
              t.identifier('something'),
              t.numericLiteral(1),
              true
            ),
            t.objectProperty(t.stringLiteral('test'), t.numericLiteral(2), true)
          ])
        )
      ]);
    });

    test('object expressions (shorthand keys)', () => {
      expectAst('{ short, hand }', [
        t.expressionStatement(
          t.objectExpression([
            t.objectProperty(
              t.identifier('short'),
              t.identifier('short'),
              false,
              true
            ),
            t.objectProperty(
              t.identifier('hand'),
              t.identifier('hand'),
              false,
              true
            )
          ])
        )
      ]);
    });

    test('multiple complex assignments', () => {
      expectAst(
        `
      let func1 = helloWorld 47 "something here" cool
      let func2 = func1 True
      `,
        [
          t.variableDeclaration('const', [
            t.variableDeclarator(
              t.identifier('func1'),
              t.callExpression(
                t.callExpression(
                  t.callExpression(t.identifier('helloWorld'), [
                    t.numericLiteral(47)
                  ]),
                  [t.stringLiteral('something here')]
                ),
                [t.identifier('cool')]
              )
            )
          ]),
          t.variableDeclaration('const', [
            t.variableDeclarator(
              t.identifier('func2'),
              t.callExpression(t.identifier('func1'), [t.booleanLiteral(true)])
            )
          ])
        ]
      );
    });

    test('function definition followed by call', () => {
      expectAst(
        `
      let fn = a => "hello, world!"

      fn 1
      `,
        [
          t.variableDeclaration('const', [
            t.variableDeclarator(
              t.identifier('fn'),
              t.arrowFunctionExpression(
                [t.identifier('a')],
                t.stringLiteral('hello, world!')
              )
            )
          ]),
          t.expressionStatement(
            t.callExpression(t.identifier('fn'), [t.numericLiteral(1)])
          )
        ]
      );
    });

    test('function definition followed by array', () => {
      expectAst(
        `
      let toStr = s => fn s

      [1, 2, 3]
      `,
        [
          t.variableDeclaration('const', [
            t.variableDeclarator(
              t.identifier('toStr'),
              t.arrowFunctionExpression(
                [t.identifier('s')],
                t.callExpression(t.identifier('fn'), [t.identifier('s')])
              )
            )
          ]),
          t.expressionStatement(
            t.arrayExpression([
              t.numericLiteral(1),
              t.numericLiteral(2),
              t.numericLiteral(3)
            ])
          )
        ]
      );
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
