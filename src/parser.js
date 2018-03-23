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
const isInterpolationStart = isSymbol('${');
const isEquals = isSymbol('=');
const isArrow = isSymbol('=>');
const isLet = isKeyword('let');

const parse = ({ source, tokens }) => {
  let index = 0;
  let token = tokens[index];

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
    const parts = [];

    while (isIdentifier(token) || isDot(token) || isLeftBrace(token)) {
      if (isIdentifier(token)) {
        parts.push({ property: t.identifier(token.value), computed: false });
      } else if (isLeftBrace(token)) {
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

  const parseSimpleIdentifier = () => {
    if (!isIdentifier(token)) return;
    advance();
    return t.identifier(token.value);
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
    advance(1);
    const fn = parseFunction(true);
    if (!fn) {
      fail('Expected a function after async keyword');
    }

    return fn;
  };

  const parseExpression = () => {
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
        return parseFunction() || parseIdentifier();
      case tokenTypes.KEYWORD:
        return parseAsyncFunction();
    }
  };

  const parseAssignment = () => {
    if (
      isLet(token) &&
      isIdentifier(tokens[index + 1]) &&
      isEquals(tokens[index + 2])
    ) {
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
