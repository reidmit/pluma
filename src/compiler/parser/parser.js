import * as nodes from '../ast-nodes';
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
    const node = new nodes.NumberNode(
      token.lineStart,
      token.lineEnd,
      token.value
    );
    advance();
    return node;
  }

  function parseBoolean() {
    if (!u.isBoolean(token)) return;
    const node = new nodes.BooleanNode(
      token.lineStart,
      token.lineEnd,
      token.value
    );
    advance();
    return node;
  }

  function parseString() {
    const literals = [];
    const expressions = [];
    const lineStart = token.lineStart;
    let lineEnd = token.lineEnd;

    while (u.isString(token) || u.isInterpolationStart(token)) {
      if (u.isString(token)) {
        literals.push(
          new nodes.StringNode(token.lineStart, token.lineEnd, token.value)
        );
      } else {
        advance();

        const innerExpression = parseExpression(token);
        if (!innerExpression) {
          fail('Invalid expression in interpolation');
        }

        expressions.push(innerExpression);
      }

      lineEnd = token.lineEnd;
      advance();
      if (!token) break;
    }

    if (literals.length === 1 && !expressions.length) {
      return literals[0];
    }

    return new nodes.InterpolatedStringNode(
      lineStart,
      lineEnd,
      literals,
      expressions
    );
  }

  function parseGetter() {
    if (!u.isDotIdentifier(token)) return;
    const node = new nodes.IdentifierNode(
      token.lineStart,
      token.lineEnd,
      token.value,
      true,
      false
    );
    advance();
    return node;
  }

  function parseSetter() {
    if (!u.isAtIdentifier(token)) return;
    const node = new nodes.IdentifierNode(
      token.lineStart,
      token.lineEnd,
      token.value,
      false,
      true
    );
    advance();
    return node;
  }

  function parseIdentifier() {
    if (!u.isIdentifier(token)) return;

    const parts = [
      new nodes.IdentifierNode(token.lineStart, token.lineEnd, token.value)
    ];

    advance();

    while (
      token &&
      token.columnStart > lastAssignmentColumn &&
      u.isDotIdentifier(token)
    ) {
      parts.push(
        new nodes.IdentifierNode(token.lineStart, token.lineEnd, token.value)
      );
      advance();
      if (!token) break;
    }

    if (parts.length === 1) return parts[0];

    return parts.reduce((expression, property) => {
      if (!expression) return property;
      return new nodes.MemberExpressionNode(
        parts[0].lineStart,
        parts[parts.length - 1].lineStart,
        parts
      );
    }, null);
  }

  function parseFunction() {
    if (!u.isIdentifier(token) || !u.isArrow(tokens[index + 1])) return;

    const param = new nodes.IdentifierNode(
      token.lineStart,
      token.lineEnd,
      token.value
    );
    advance(2);
    const body = parseExpression();
    if (!body) {
      fail('Expected a valid expression in function body');
    }

    return new nodes.FunctionNode(param.lineStart, body.lineEnd, param, body);
  }

  function parsePossibleCallExpression() {
    let func =
      parseFunction() || parseGetter() || parseSetter() || parseIdentifier();
    if (!func) return;
    let arg;
    while (token && token.columnStart > lastAssignmentColumn) {
      const lineStart = token.lineStart;
      const lineEnd = token.lineEnd;
      arg = parseExpression();
      if (!arg) break;
      func = new nodes.CallNode(lineStart, lineEnd, func, arg);
    }
    return func;
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
      const key = new nodes.IdentifierNode(
        token.lineStart,
        token.lineEnd,
        token.value
      );
      advance(2);
      const value = parseExpression();
      if (!value) {
        fail('Expected a valid expression after :');
      }
      return new nodes.ObjectPropertyNode(
        key.lineStart,
        value.lineEnd,
        key,
        value
      );
    }

    if (
      u.isIdentifier(token) &&
      (u.isComma(tokens[index + 1]) || u.isRightBracket(tokens[index + 1]))
    ) {
      const key = new nodes.IdentifierNode(
        token.lineStart,
        token.lineEnd,
        token.value
      );
      advance();
      return new nodes.ObjectPropertyNode(key.lineStart, key.lineEnd, key, key);
    }
  }

  function parseObject() {
    if (!u.isLeftBracket(token)) return;
    const lineStart = token.lineStart;
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

    const lineEnd = token.lineEnd;
    advance();
    return new nodes.ObjectNode(lineStart, lineEnd, properties);
  }

  function parseArray() {
    if (!u.isLeftBrace(token)) return;
    const lineStart = token.lineStart;
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

    const lineEnd = token.lineEnd;
    advance();
    return new nodes.ArrayNode(lineStart, lineEnd, elements);
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
      case tokenTypes.DOT_IDENTIFIER:
      case tokenTypes.AT_IDENTIFIER:
      case tokenTypes.IDENTIFIER:
        return parsePossibleCallExpression();
      case tokenTypes.SYMBOL:
        return parseParenthetical() || parseObject() || parseArray();
    }
  }

  function parseAssignment() {
    if (!u.isLet(token)) return;
    const lineStart = token.lineStart;

    if (u.isIdentifier(tokens[index + 1]) && u.isEquals(tokens[index + 2])) {
      lastAssignmentColumn = token.columnStart;
      advance();
      const id = new nodes.IdentifierNode(
        token.lineStart,
        token.lineEnd,
        token.value
      );
      advance(2);
      const valueExpression = parseExpression(token);
      if (!valueExpression) {
        fail('Expected a valid expression on right-hand side of assignment');
      }

      return new nodes.AssignmentNode(
        lineStart,
        valueExpression.lineEnd,
        id,
        valueExpression
      );
    }

    fail(
      t => `Unexpected ${t} found after "let" keyword. Expected an identifier.`,
      tokens[index + 1]
    );
  }

  function parseStatement() {
    const expression = parseExpression(token);
    if (expression) return expression;

    const assignment = parseAssignment(token);
    if (assignment) return assignment;
  }

  const body = [];
  while (index < tokens.length) {
    const node = parseStatement(token);
    if (node) body.push(node);
    else advance();
  }

  const firstLine = body.length ? body[0].lineStart : 1;
  const lastLine = body.length ? body[body.length - 1].lineEnd : 1;

  return new nodes.ModuleNode(firstLine, lastLine, body);
}

export default parse;
