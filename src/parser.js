import * as t from 'babel-types';
import { tokenTypes } from './constants';

const parse = ({ source, tokens }) => {
  let i = 0;

  const fail = message => {
    // TODO: improve with tokens/line numbers/etc
    throw new Error(`Parse error: ${message}`);
  };

  const parseNumber = token => {
    i++;
    return t.numericLiteral(token.value);
  };

  const parseBoolean = token => {
    i++;
    return t.booleanLiteral(token.value);
  };

  const parseString = token => {
    const stringParts = [];
    const expressions = [];

    while (
      token.type === tokenTypes.STRING ||
      (token.type === tokenTypes.SYMBOL && token.value === '${')
    ) {
      if (token.type === tokenTypes.STRING) {
        stringParts.push(t.templateElement(token.value, false));
      } else {
        token = tokens[++i];
        const innerExpression = parseExpression(token);
        if (!innerExpression) {
          fail('Invalid expression in interpolation');
        } else {
          expressions.push(innerExpression);
        }
      }

      token = tokens[++i];
      if (!token) break;
    }

    if (stringParts.length === 1 && !expressions.length) {
      return t.stringLiteral(stringParts[0].value);
    }

    stringParts[stringParts.length - 1].tail = true;
    return t.templateLiteral(stringParts, expressions);
  };

  const parseIdentifier = token => {
    i++;
    return t.identifier(token.value);
  };

  const parseExpression = token => {
    switch (token.type) {
      case tokenTypes.NUMBER:
        return parseNumber(token);
      case tokenTypes.BOOLEAN:
        return parseBoolean(token);
      case tokenTypes.STRING:
        return parseString(token);
      case tokenTypes.IDENTIFIER:
        return parseIdentifier(token);
    }
  };

  const parseStatement = token => {
    const expression = parseExpression(token);
    if (expression) {
      return t.expressionStatement(expression);
    }
  };

  const body = [];
  let TODO = 0;
  while (i < tokens.length && TODO < 1000) {
    const node = parseStatement(tokens[i]);
    if (node) {
      body.push(node);
    }

    TODO++; //TODO: remove
  }

  return t.file(t.program(body), [], tokens);
};

export { parse };
