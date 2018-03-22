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

  test('a single number', () => {
    expectAst('47', [t.expressionStatement(t.numericLiteral(47))]);
  });

  test('a single boolean', () => {
    expectAst('true', [t.expressionStatement(t.booleanLiteral(true))]);
  });

  test('a single identifier', () => {
    expectAst('lol', [t.expressionStatement(t.identifier('lol'))]);
  });

  test('a single string', () => {
    expectAst(`'hello, world!'`, [
      t.expressionStatement(t.stringLiteral('hello, world!'))
    ]);
  });

  test('a single, interpolated string', () => {
    expectAst(`'hello, \${name}!'`, [
      t.expressionStatement(
        t.templateLiteral(
          [t.templateElement('hello, ', false), t.templateElement('!', true)],
          [t.identifier('name')]
        )
      )
    ]);
  });
});
