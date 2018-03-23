import { parse } from '../src/parser';
import { tokenize } from '../src/tokenizer';
import * as t from 'babel-types';

const expectAst = (source, bodyNodes) => {
  const tokens = tokenize({ source });
  expect(parse({ source, tokens })).toEqual(
    t.file(t.program(bodyNodes), [], tokens)
  );
};

describe('parser', () => {
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
    expectAst('true', [t.expressionStatement(t.booleanLiteral(true))]);
  });

  test('identifier', () => {
    expectAst('lol', [t.expressionStatement(t.identifier('lol'))]);
  });

  test('string literal', () => {
    expectAst('\'hello, world!\'', [
      t.expressionStatement(t.stringLiteral('hello, world!'))
    ]);
  });

  test('interpolated string literal', () => {
    expectAst('\'hello, ${name}!\'', [
      t.expressionStatement(
        t.templateLiteral(
          [t.templateElement('hello, ', false), t.templateElement('!', true)],
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
    expectAst('a[b][\'hello\'][0]', [
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
      let someString = 'hello, world!'
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
});
