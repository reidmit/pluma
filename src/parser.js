import * as t from 'babel-types';
import { tokenTypes } from './constants';

const isType = type => token => token.type === tokenTypes[type];
const isSymbol = value => token =>
  token.type === tokenTypes.SYMBOL && token.value === value;
const isString = isType('STRING');
const isNumber = isType('NUMBER');
const isIdentifier = isType('IDENTIFIER');
const isDot = isSymbol('.');
const isLeftBrace = isSymbol('[');
const isRightBrace = isSymbol(']');
const isInterpolationStart = isSymbol('${');

const parse = ({ source, tokens }) => {
  let index = 0;
  let token = tokens[index];

  const advance = (n = 1) => {
    token = tokens[++index];
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

  const parseExpression = () => {
    switch (token.type) {
      case tokenTypes.NUMBER:
        return parseNumber(token);
      case tokenTypes.BOOLEAN:
        return parseBoolean(token);
      case tokenTypes.STRING:
        return parseString(token);
      case tokenTypes.NULL:
        return parseNull(token);
      case tokenTypes.IDENTIFIER:
        return parseIdentifier(token);
    }
  };

  const parseStatement = () => {
    const expression = parseExpression(token);
    if (expression) {
      return t.expressionStatement(expression);
    }
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
