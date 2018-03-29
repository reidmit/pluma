import * as t from 'babel-types';
import * as u from '../util';
import { ParserError } from '../errors';
import { tokenToString } from '../errors/error-helper';
import { tokenTypes } from '../constants';

function parse({ source, tokens }) {
  let index = 0;
  let token = tokens[index];
  let lastAssignmentColumn = 0;

  function advance(n = 1) {
    index += n;
    token = tokens[index];
  }

  function fail(message, badToken = token) {
    message =
      typeof message === 'function'
        ? message(tokenToString(badToken))
        : message;
    throw new ParserError(message, source, badToken);
  }

  function parseNumber() {
    if (!u.isNumber(token)) return;
    const node = t.numericLiteral(token.value);
    advance();
    return node;
  }

  function parseBoolean() {
    if (!u.isBoolean(token)) return;
    const node = t.booleanLiteral(token.value);
    advance();
    return node;
  }

  function parseNull() {
    if (!u.isNull(token)) return;
    advance();
    return t.nullLiteral();
  }

  function parseString() {
    const stringParts = [];
    const expressions = [];

    while (u.isString(token) || u.isInterpolationStart(token)) {
      if (u.isString(token)) {
        stringParts.push(
          t.templateElement({ raw: token.value, cooked: token.value }, false)
        );
      } else {
        advance();

        const innerExpression = parseExpression(token);
        if (!innerExpression) {
          fail('Invalid expression in interpolation');
        }

        expressions.push(innerExpression);
      }

      advance();
      if (!token) break;
    }

    if (stringParts.length === 1 && !expressions.length) {
      return t.stringLiteral(stringParts[0].value.raw);
    }

    stringParts[stringParts.length - 1].tail = true;
    return t.templateLiteral(stringParts, expressions);
  }

  function parseIdentifier() {
    if (!u.isIdentifier(token)) return;

    const parts = [t.identifier(token.value)];
    advance();

    while (
      token &&
      token.columnStart > lastAssignmentColumn &&
      u.isDot(token)
    ) {
      advance();
      if (!u.isIdentifier(token)) {
        fail('Unexpected token after dot');
      }
      parts.push(t.identifier(token.value));
      advance();
      if (!token) break;
    }

    if (parts.length === 1) return parts[0];

    return parts.reduce((expression, property) => {
      if (!expression) return property;
      return t.memberExpression(expression, property);
    }, null);
  }

  function parseFunction() {
    if (!u.isIdentifier(token) || !u.isArrow(tokens[index + 1])) return;

    const params = [t.identifier(token.value)];
    advance(2);
    const body = parseExpression();
    if (!body) {
      fail('Expected a valid expression in function body');
    }

    return t.arrowFunctionExpression(params, body);
  }

  function parsePossibleCallExpression() {
    let left = parseFunction() || parseIdentifier();
    if (!left) return;
    let right;
    while (
      token &&
      token.columnStart > lastAssignmentColumn &&
      (right = parseExpression())
    ) {
      left = t.callExpression(left, [right]);
    }
    return left;
  }

  function parseParenthetical() {
    if (!u.isLeftParen(token)) return;
    const leftParen = token;

    advance();
    const expression = parseExpression();
    if (!token) {
      fail('Unexpectedly reached end of input.', tokens[index - 1]);
    }

    if (!expression) {
      fail('Expected expression after (');
    }

    if (!u.isRightParen(token)) {
      fail(
        `Missing closing ")" to match opening "(" at line ${
          leftParen.lineStart
        }, column ${leftParen.columnStart}.`,
        leftParen
      );
    }
    advance();
    return expression;
  }

  function parseObjectProperty() {
    if (u.isIdentifier(token) && u.isColon(tokens[index + 1])) {
      const key = t.identifier(token.value);
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value);
    }

    if (
      u.isIdentifier(token) &&
      (u.isComma(tokens[index + 1]) || u.isRightBracket(tokens[index + 1]))
    ) {
      const key = t.identifier(token.value);
      advance();
      return t.objectProperty(key, key, false, true);
    }

    if (u.isString(token) && u.isColon(tokens[index + 1])) {
      const key = t.stringLiteral(token.value);
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value);
    }

    if (u.isLeftBrace(token)) {
      advance();
      const key = parseExpression();
      if (!key) {
        fail('Could not parse computed key');
      }
      if (!u.isRightBrace(token)) {
        fail('Expected a closing ] after computed key');
      }
      advance();
      if (!u.isColon(token)) {
        fail('Expected a : after computed key');
      }
      advance();
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value, true);
    }
  }

  function parseObject() {
    if (!u.isLeftBracket(token)) return;
    advance();

    const properties = [];
    while (!u.isRightBracket(token)) {
      const property = parseObjectProperty();
      if (!property) {
        fail('Failed to parse object property');
      }
      properties.push(property);
      if (u.isComma(token)) {
        advance();
      } else if (!u.isRightBracket(token)) {
        fail('Missing right bracket }');
      }
    }

    advance();
    return t.objectExpression(properties);
  }

  function parseArray() {
    if (!u.isLeftBrace(token)) return;
    const leftBrace = token;
    advance();

    const elements = [];
    while (!u.isRightBrace(token)) {
      const expr = parseExpression();
      if (!expr) {
        fail('Invalid expression in array');
      }
      elements.push(expr);
      if (u.isComma(token)) {
        advance();
      } else if (!u.isRightBrace(token)) {
        fail(
          `Missing closing "]" to match opening "[" at line ${
            leftBrace.lineStart
          }, column ${leftBrace.columnStart}.`,
          leftBrace
        );
      }
    }

    advance();
    return t.arrayExpression(elements);
  }

  function parseExpression() {
    if (!token) return;

    switch (token.type) {
      case tokenTypes.NUMBER:
        return parseNumber();
      case tokenTypes.BOOLEAN:
        return parseBoolean();
      case tokenTypes.STRING:
        return parseString();
      case tokenTypes.NULL:
        return parseNull();
      case tokenTypes.IDENTIFIER:
        return parsePossibleCallExpression();
      case tokenTypes.SYMBOL:
        return parseParenthetical() || parseObject() || parseArray();
    }
  }

  function parseAssignment() {
    if (!u.isLet(token)) return;

    if (u.isIdentifier(tokens[index + 1]) && u.isEquals(tokens[index + 2])) {
      lastAssignmentColumn = token.columnStart;
      advance();
      const id = t.identifier(token.value);
      advance(2);
      const valueExpression = parseExpression(token);
      if (!valueExpression) {
        fail('Expected a valid expression on right-hand side of assignment');
      }

      return t.variableDeclaration('const', [
        t.variableDeclarator(id, valueExpression)
      ]);
    }

    fail(
      t => `Unexpected ${t} found after "let" keyword. Expected an identifier.`,
      tokens[index + 1]
    );
  }

  function parseStatement() {
    const expression = parseExpression(token);
    if (expression) return t.expressionStatement(expression);

    const assignment = parseAssignment(token);
    if (assignment) return assignment;
  }

  const body = [];
  while (index < tokens.length) {
    const node = parseStatement(token);
    if (node) body.push(node);
    else advance();
  }

  return t.file(t.program(body), [], tokens);
}

export default parse;
