import * as t from 'babel-types';
import { tokenTypes } from './constants';

const isType = type => token => token && token.type === tokenTypes[type];
const isSymbol = value => token =>
  token && token.type === tokenTypes.SYMBOL && token.value === value;
const isKeyword = value => token =>
  token && token.type === tokenTypes.KEYWORD && token.value === value;
const isString = isType('STRING');
const isNumber = isType('NUMBER');
const isIdentifier = isType('IDENTIFIER');
const isDot = isSymbol('.');
const isLeftBrace = isSymbol('[');
const isRightBrace = isSymbol(']');
const isLeftParen = isSymbol('(');
const isRightParen = isSymbol(')');
const isLeftBracket = isSymbol('{');
const isRightBracket = isSymbol('}');
const isInterpolationStart = isSymbol('${');
const isEquals = isSymbol('=');
const isArrow = isSymbol('=>');
const isComma = isSymbol(',');
const isColon = isSymbol(':');
const isLet = isKeyword('let');

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
    if (!isNumber(token)) return;
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

    while (isString(token) || isInterpolationStart(token)) {
      if (isString(token)) {
        stringParts.push(t.templateElement(token.value, false));
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
      return t.stringLiteral(stringParts[0].value);
    }

    stringParts[stringParts.length - 1].tail = true;
    return t.templateLiteral(stringParts, expressions);
  };

  const parseIdentifier = () => {
    if (!isIdentifier(token)) return;
    const parts = [{ property: t.identifier(token.value), computed: false }];
    advance();

    while (isDot(token) || isLeftBrace(token)) {
      if (isDot(token)) {
        advance();
        if (!isIdentifier(token)) {
          fail('Unexpected token after dot');
        }
        parts.push({ property: t.identifier(token.value), computed: false });
      } else {
        advance();
        const innerExpression = parseExpression(token);

        if (!innerExpression) {
          fail('Invalid expression in brackets');
        }

        if (!isRightBrace(token)) {
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
    if (!isIdentifier(token) || !isArrow(tokens[index + 1])) return;
    const params = [t.identifier(token.value)];
    advance(2);
    const body = parseExpression();
    if (!body) {
      fail('Expected a valid expression in function body');
    }
    return t.arrowFunctionExpression(params, body, async);
  };

  const parseAsyncFunction = () => {
    if (!isKeyword('async')(token)) return;
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
    if (!isLeftParen(token)) return;
    advance();
    const expression = parseExpression();
    if (!expression) {
      fail('Expected expression after (');
    }
    if (!isRightParen(token)) {
      fail('Missing closing )');
    }
    advance();
    return expression;
  };

  const parseObjectProperty = () => {
    if (isIdentifier(token) && isColon(tokens[index + 1])) {
      const key = t.identifier(token.value);
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value);
    } else if (
      isIdentifier(token) &&
      (isComma(tokens[index + 1]) || isRightBracket(tokens[index + 1]))
    ) {
      const key = t.identifier(token.value);
      advance();
      return t.objectProperty(key, key, false, true);
    } else if (isString(token) && isColon(tokens[index + 1])) {
      const key = t.stringLiteral(token.value);
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return t.objectProperty(key, value);
    } else if (isLeftBrace(token)) {
      advance();
      const key = parseExpression();
      if (!key) {
        fail('Could not parse computed key');
      }
      if (!isRightBrace(token)) {
        fail('Expected a closing ] after computed key');
      }
      advance();
      if (!isColon(token)) {
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
    if (!isLeftBracket(token)) return;
    advance();
    const properties = [];
    while (!isRightBracket(token)) {
      const property = parseObjectProperty();
      if (!property) {
        fail('Failed to parse object property');
      }
      properties.push(property);
      if (isComma(token)) {
        advance();
      } else if (!isRightBracket(token)) {
        fail('Missing right bracket }');
      }
    }
    return t.objectExpression(properties);
  };

  const parseArray = () => {
    if (!isLeftBrace(token)) return;
    advance();
    const elements = [];
    while (!isRightBrace(token)) {
      const expr = parseExpression();
      if (!expr) {
        fail('Invalid expression in array');
      }
      elements.push(expr);
      if (isComma(token)) {
        advance();
      } else if (!isRightBrace(token)) {
        fail('Missing right brace ]');
      }
    }

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
      isLet(token) &&
      isIdentifier(tokens[index + 1]) &&
      isEquals(tokens[index + 2])
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
  let TODO = 0;
  while (index < tokens.length && TODO < 1000) {
    const node = parseStatement(token);
    if (node) body.push(node);

    TODO++; //TODO: remove
  }

  return t.file(t.program(body), [], tokens);
};

export { parse };
