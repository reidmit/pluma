import * as t from 'babel-types';
import * as u from './util';
import { tokenTypes } from './constants';

const parse = ({ source, tokens }) => {
  let index = 0;
  let token = tokens[index];
  let lastAssignmentColumn = 0;

  const advance = (n = 1) => {
    index += n;
    token = tokens[index];
  };

  const fail = message => {
    // TODO: improve with tokens/line numbers/etc
    throw new Error(`Parse error: ${message}`);
  };

  const parseNumber = () => {
    if (!u.isNumber(token)) return;
    const node = t.numericLiteral(token.value);
    advance();
    return node;
  };

  const parseBoolean = () => {
    const node = t.booleanLiteral(token.value);
    advance();
    return node;
  };

  const parseNull = () => {
    advance();
    return t.nullLiteral();
  };

  const parseString = () => {
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
  };

  const parseIdentifier = () => {
    if (!u.isIdentifier(token)) return;
    const parts = [{ property: t.identifier(token.value), computed: false }];
    advance();

    while (
      token &&
      token.columnStart > lastAssignmentColumn &&
      (u.isDot(token) || u.isLeftBrace(token))
    ) {
      if (u.isDot(token)) {
        advance();
        if (!u.isIdentifier(token)) {
          fail('Unexpected token after dot');
        }
        parts.push({ property: t.identifier(token.value), computed: false });
      } else {
        advance();
        const innerExpression = parseExpression(token);

        if (!innerExpression) {
          fail('Invalid expression in brackets');
        }

        if (!u.isRightBrace(token)) {
          fail('Missing closing ]');
        }

        parts.push({ property: innerExpression, computed: true });
      }

      advance();
      if (!token) break;
    }

    if (parts.length === 1) return parts[0].property;

    return parts.reduce((expression, { property, computed }) => {
      if (!expression) return property;
      return t.memberExpression(expression, property, computed);
    }, null);
  };

  const parseFunction = (async = false) => {
    if (!u.isIdentifier(token) || !u.isArrow(tokens[index + 1])) return;
    const params = [t.identifier(token.value)];
    advance(2);
    const body = parseExpression();
    if (!body) {
      fail('Expected a valid expression in function body');
    }
    return t.arrowFunctionExpression(params, body, async);
  };

  const parseAsyncFunction = () => {
    if (!u.isKeyword('async')(token)) return;
    advance();
    const fn = parseFunction(true);
    if (!fn) {
      fail('Expected a function after async keyword');
    }

    return fn;
  };

  const parsePossibleCallExpression = () => {
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
  };

  const parseParenthetical = () => {
    if (!u.isLeftParen(token)) return;
    advance();
    const expression = parseExpression();
    if (!expression) {
      fail('Expected expression after (');
    }
    if (!u.isRightParen(token)) {
      fail('Missing closing )');
    }
    advance();
    return expression;
  };

  const parseObjectProperty = () => {
    if (u.isIdentifier(token) && u.isColon(tokens[index + 1])) {
      const key = t.identifier(token.value);
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value);
    } else if (
      u.isIdentifier(token) &&
      (u.isComma(tokens[index + 1]) || u.isRightBracket(tokens[index + 1]))
    ) {
      const key = t.identifier(token.value);
      advance();
      return t.objectProperty(key, key, false, true);
    } else if (u.isString(token) && u.isColon(tokens[index + 1])) {
      const key = t.stringLiteral(token.value);
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value);
    } else if (u.isLeftBrace(token)) {
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
  };

  const parseObject = () => {
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
  };

  const parseArray = () => {
    if (!u.isLeftBrace(token)) return;
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
        fail('Missing right brace ]');
      }
    }

    advance();
    return t.arrayExpression(elements);
  };

  const parseExpression = () => {
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
      case tokenTypes.KEYWORD:
        return parseAsyncFunction();
      case tokenTypes.SYMBOL:
        return parseParenthetical() || parseObject() || parseArray();
    }
  };

  const parseAssignment = () => {
    if (
      u.isLet(token) &&
      u.isIdentifier(tokens[index + 1]) &&
      u.isEquals(tokens[index + 2])
    ) {
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
  };

  const parseStatement = () => {
    const expression = parseExpression(token);
    if (expression) {
      return t.expressionStatement(expression);
    }

    const assignment = parseAssignment(token);
    if (assignment) return assignment;
  };

  const body = [];
  while (index < tokens.length) {
    const node = parseStatement(token);
    if (node) body.push(node);
  }

  return t.file(t.program(body), [], tokens);
};

export default parse;
