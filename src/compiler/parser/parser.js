import * as u from '../util';
import { buildNode } from '../ast-nodes';
import { ParserError } from '../errors';
import { tokenToString } from '../errors/error-helper';
import { tokenTypes } from '../constants';

function parse({ source, tokens }) {
  let index = 0;
  let token = tokens[index];
  let lastAssignmentColumn = 0;
  const fullLineComments = {};

  function advance(amount = 1) {
    index += amount;
    token = tokens[index];
  }

  function fail(message, badToken = token) {
    message =
      typeof message === 'function'
        ? message(badToken ? tokenToString(badToken) : 'end of input')
        : message;

    throw new ParserError(
      message,
      source,
      badToken || tokens[tokens.length - 1]
    );
  }

  function collectComments(node) {
    const { lineStart } = node;
    const nodeComments = [];

    let lineNumber = lineStart - 1;
    while (fullLineComments[lineNumber]) {
      nodeComments.push(fullLineComments[lineNumber]);
      lineNumber--;
    }

    node.comments = nodeComments.reverse();
    node.lineStart = lineNumber + 1;
  }

  function parseNumber() {
    if (!u.isNumber(token)) return;

    const node = buildNode.Number(token.lineStart, token.lineEnd)({
      value: token.value
    });

    advance();

    return node;
  }

  function parseBoolean() {
    if (!u.isBoolean(token)) return;

    const node = buildNode.Boolean(token.lineStart, token.lineEnd)({
      value: token.value
    });

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
          buildNode.String(token.lineStart, token.lineEnd)({
            value: token.value
          })
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

    return buildNode.InterpolatedString(lineStart, lineEnd)({
      literals,
      expressions
    });
  }

  function parseGetter() {
    if (!u.isDotIdentifier(token)) return;

    const node = buildNode.Identifier(token.lineStart, token.lineEnd)({
      value: token.value,
      isGetter: true,
      isSetter: false
    });

    advance();

    return node;
  }

  function parseSetter() {
    if (!u.isAtIdentifier(token)) return;

    const node = buildNode.Identifier(token.lineStart, token.lineEnd)({
      value: token.value,
      isGetter: false,
      isSetter: true
    });

    advance();

    return node;
  }

  function parseIdentifier() {
    if (!u.isIdentifier(token)) return;

    const parts = [
      buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      })
    ];

    advance();

    while (
      token &&
      token.columnStart > lastAssignmentColumn &&
      u.isDotIdentifier(token)
    ) {
      parts.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();

      if (!token) break;
    }

    if (parts.length === 1) return parts[0];

    return parts.reduce((expression, property) => {
      if (!expression) return property;

      return buildNode.MemberExpression(
        parts[0].lineStart,
        parts[parts.length - 1].lineStart
      )({
        parts
      });
    }, null);
  }

  function parseFunction() {
    if (!u.isIdentifier(token) || !u.isArrow(tokens[index + 1])) return;

    const parameter = buildNode.Identifier(token.lineStart, token.lineEnd)({
      value: token.value,
      isGetter: false,
      isSetter: false
    });

    advance(2);

    const body = parseExpression();

    if (!body) {
      fail('Expected a valid expression in function body');
    }

    return buildNode.Function(parameter.lineStart, body.lineEnd)({
      parameter,
      body
    });
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

      func = buildNode.Call(lineStart, lineEnd)({ callee: func, arg });
    }

    return func;
  }

  function parseParenthetical() {
    if (!u.isLeftParen(token)) return;

    const leftParen = token;
    const lineStart = token.lineStart;
    const expressions = [];

    advance();

    const expression = parseExpression();

    if (!token) {
      fail('Unexpectedly reached end of input.', tokens[index - 1]);
    }

    if (!expression) {
      fail('Expected expression after (');
    }

    expressions.push(expression);

    while (u.isComma(token)) {
      advance();

      const tupleElement = parseExpression();

      if (!token) {
        fail('Unexpectedly reached end of input.', tokens[index - 1]);
      }

      if (!tupleElement) {
        fail('Expected a valid expression after "," in tuple.');
      }

      expressions.push(tupleElement);
    }

    if (!u.isRightParen(token)) {
      fail(
        t =>
          `Expected closing ")" to match opening "(" at line ${
            leftParen.lineStart
          }, column ${leftParen.columnStart}, but found ${t} instead.`
      );
    }

    advance();

    if (expressions.length === 1) return expressions[0];

    return buildNode.Tuple(
      lineStart,
      expressions[expressions.length - 1].lineEnd
    )({ entries: expressions });
  }

  function parseObjectProperty() {
    if (u.isIdentifier(token) && u.isColon(tokens[index + 1])) {
      const key = buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance(2);

      const value = parseExpression();

      if (!value) {
        fail('Expected a valid expression after :');
      }

      return buildNode.ObjectProperty(key.lineStart, value.lineEnd)({
        key,
        value
      });
    }

    if (
      u.isIdentifier(token) &&
      (u.isComma(tokens[index + 1]) || u.isRightBrace(tokens[index + 1]))
    ) {
      const key = buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance();

      return buildNode.ObjectProperty(key.lineStart, key.lineEnd)({
        key,
        value: key
      });
    }
  }

  function parseObject() {
    if (!u.isLeftBrace(token)) return;

    const lineStart = token.lineStart;
    const columnStart = token.columnStart;

    advance();

    const properties = [];

    while (!u.isRightBrace(token)) {
      const property = parseObjectProperty();

      if (!property) {
        fail('Failed to parse object property.');
      }

      properties.push(property);

      if (u.isComma(token)) {
        advance();
      } else if (!u.isRightBrace(token)) {
        fail(
          t =>
            `Expected closing "}" to match opening "{" at line ${lineStart}, column ${columnStart}, but found ${t} instead.`
        );
      }
    }

    const lineEnd = token.lineEnd;

    advance();

    return buildNode.Object(lineStart, lineEnd)({ properties });
  }

  function parseArray() {
    if (!u.isLeftBracket(token)) return;

    const lineStart = token.lineStart;
    const leftBracket = token;

    advance();

    const elements = [];

    while (!u.isRightBracket(token)) {
      const expr = parseExpression();

      if (!expr) {
        fail('Invalid expression in array');
      }

      elements.push(expr);

      if (u.isComma(token)) {
        advance();
      } else if (!u.isRightBracket(token)) {
        fail(
          t =>
            `Expected closing "]" to match opening "[" at line ${
              leftBracket.lineStart
            }, column ${leftBracket.columnStart}, but found ${t} instead.`
        );
      }
    }

    const lineEnd = token.lineEnd;

    advance();

    return buildNode.Array(lineStart, lineEnd)({ elements });
  }

  function parseConditional() {
    if (!u.isIf(token)) return;

    const lineStart = token.lineStart;

    advance();

    const predicate = parseExpression();

    if (!predicate) {
      fail('Expected to find a valid expression after "if" keyword.');
    }

    if (!u.isThen(token)) {
      fail(
        'Expected to find a "then" keyword, followed by a then case, in conditional expression.'
      );
    }

    advance();

    const thenCase = parseExpression();

    if (!thenCase) {
      fail(
        'Expected to find a valid expression after "then" keyword in conditional expression.'
      );
    }

    if (!u.isElse(token)) {
      fail(
        'Expected to find an "else" keyword, followed by an else case, in conditional expression.'
      );
    }

    advance();

    const elseCase = parseExpression();

    if (!elseCase) {
      fail(
        'Expected to find a valid expression after "else" keyword in conditional expression.'
      );
    }

    return buildNode.Conditional(lineStart, elseCase.lineEnd)({
      predicate,
      thenCase,
      elseCase
    });
  }

  function parseTypeExpression() {
    if (!token) return;

    let firstNode;

    if (u.isLeftParen(token)) {
      const leftParen = token;

      advance();

      firstNode = parseTypeExpression();

      if (u.isComma(token)) {
        const otherTupleEntries = [];

        while (u.isComma(token)) {
          advance();

          const tupleEntry = parseTypeExpression();

          if (!tupleEntry) {
            fail(
              'Expected a valid type expression after "," in tuple type expression.'
            );
          }

          otherTupleEntries.push(tupleEntry);
        }

        firstNode = buildNode.TypeTuple(leftParen.lineStart, leftParen.lineEnd)(
          { typeEntries: [firstNode, ...otherTupleEntries] }
        );
      }

      if (!u.isRightParen(token)) {
        fail(
          t =>
            `Expected closing ")" to match opening "(" at line ${
              leftParen.lineStart
            }, column ${leftParen.columnStart}, but found ${t} instead.`
        );
      }

      firstNode.lineEnd = token.lineEnd;

      advance();
    } else if (u.isLeftBrace(token)) {
      const lineStart = token.lineStart;

      advance();

      const entries = [];

      while (u.isIdentifier(token)) {
        const name = buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        });

        advance();

        if (!u.isDoubleColon(token)) {
          fail(
            t =>
              `Expected "::" after entry name in record type, but found ${t} instead.`
          );
        }

        advance();

        const value = parseTypeExpression();

        if (!value) {
          fail('Expected valid type expression after "::" in record type.');
        }

        entries.push(
          buildNode.RecordTypeEntry(name.lineStart, value.lineEnd)({
            name,
            typeExpression: value
          })
        );

        if (u.isComma(token)) {
          advance();

          continue;
        }

        if (u.isRightBrace(token)) {
          advance();

          break;
        }

        fail(t => `Unexpected ${t} found in record type.`);
      }

      firstNode = buildNode.RecordType(
        lineStart,
        entries[entries.length - 1].lineEnd
      )({
        entries
      });
    } else if (u.isIdentifier(token) && /^[a-z]/.test(token.value)) {
      const id = buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance();

      firstNode = buildNode.TypeVariable(id.lineStart, id.lineEnd)({
        typeName: id
      });
    } else if (u.isIdentifier(token) && /^[A-Z]/.test(token.value)) {
      const tagName = buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance();

      const typeExpression = parseTypeExpression() || null;

      firstNode = buildNode.TypeTag(
        tagName.lineStart,
        (typeExpression || tagName).lineEnd
      )({
        typeTagName: tagName,
        typeExpression
      });
    }

    if (firstNode && u.isThinArrow(token)) {
      advance();

      const rightSide = parseTypeExpression();

      if (!rightSide) {
        fail(
          t =>
            `Expected valid type expression after "->" in type alias declaration, but found ${t} instead.`
        );
      }

      return buildNode.TypeFunction(firstNode.lineStart, rightSide.lineEnd)({
        from: firstNode,
        to: rightSide
      });
    }

    return firstNode;
  }

  function parseTypeAliasDeclaration() {
    if (!u.isType(token)) return;

    const lineStart = token.lineStart;

    advance();

    if (!u.isAlias(token)) {
      fail('Expected keyword "alias" in type alias declaration');
    }

    advance();

    if (!u.isIdentifier(token)) {
      fail(
        t =>
          `Expected to find a type name after "type" keyword, but found ${t} instead.`,
        token
      );
    }

    if (!/^[A-Z]/.test(token.value)) {
      fail(
        `Type names must start with an uppercase letter, but "${
          token.value
        }" does not. Did you mean "${u.capitalize(token.value)}"?`,
        token
      );
    }

    const typeName = buildNode.Identifier(token.lineStart, token.lineEnd)({
      value: token.value,
      isGetter: false,
      isSetter: false
    });

    advance();

    const typeParameters = [];
    while (u.isIdentifier(token)) {
      typeParameters.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();
    }

    if (!u.isEquals(token)) {
      fail(
        t =>
          `Expected symbol "=" in type alias declaration, but found ${t} instead.`,
        token
      );
    }

    advance();

    const typeExpression = parseTypeExpression();

    if (!typeExpression) {
      fail(
        t =>
          `Expected a valid type expression after "=" in type alias declaration, but found ${t} instead.`
      );
    }

    return buildNode.TypeAliasDeclaration(lineStart, typeExpression.lineEnd)({
      typeName,
      typeParameters,
      typeExpression
    });
  }

  function parseTypeDeclaration() {
    if (!u.isType(token)) return;

    if (u.isAlias(tokens[index + 1])) return parseTypeAliasDeclaration();

    const lineStart = token.lineStart;

    advance();

    if (!u.isIdentifier(token)) {
      fail(
        t =>
          `Expected to find a type name after "type" keyword, but found ${t} instead.`,
        token
      );
    }

    if (!/^[A-Z]/.test(token.value)) {
      fail(
        `Type names must start with an uppercase letter, but "${
          token.value
        }" does not. Did you mean "${u.capitalize(token.value)}"?`,
        token
      );
    }

    const typeName = buildNode.Identifier(token.lineStart, token.lineEnd)({
      value: token.value,
      isGetter: false,
      isSetter: false
    });

    advance();

    const typeParameters = [];
    while (u.isIdentifier(token)) {
      typeParameters.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();
    }

    if (!u.isEquals(token)) {
      fail(
        t => `Expected symbol "=" in type declaration, but found ${t} instead.`,
        token
      );
    }

    advance();

    const typeConstructors = [];

    if (!u.isIdentifier(token)) {
      fail(
        t =>
          `Expected to find a type constructor name in type declaration, but found ${t} instead.`,
        token
      );
    }

    const firstConstructorName = buildNode.Identifier(
      token.lineStart,
      token.lineEnd
    )({
      value: token.value,
      isGetter: false,
      isSetter: false
    });

    advance();

    const firstConstructorParams = [];

    while (u.isIdentifier(token)) {
      firstConstructorParams.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();
    }

    typeConstructors.push(
      buildNode.TypeConstructor(
        firstConstructorName.lineStart,
        firstConstructorParams.length
          ? firstConstructorParams[firstConstructorParams.length - 1].lineEnd
          : firstConstructorName.lineEnd
      )({
        typeName: firstConstructorName,
        typeParameters: firstConstructorParams
      })
    );

    while (token && u.isBar(token)) {
      advance();

      if (!u.isIdentifier(token)) {
        fail(
          t =>
            `Expected to find a type constructor name after "|" in type declaration, but found ${t} instead.`
        );
      }

      const constructorName = buildNode.Identifier(
        token.lineStart,
        token.lineEnd
      )({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance();

      const constructorParams = [];

      while (token && u.isIdentifier(token)) {
        constructorParams.push(
          buildNode.Identifier(token.lineStart, token.lineEnd)({
            value: token.value,
            isGetter: false,
            isSetter: false
          })
        );

        advance();
      }

      typeConstructors.push(
        buildNode.TypeConstructor(
          constructorName.lineStart,
          constructorParams.length
            ? constructorParams[constructorParams.length - 1].lineEnd
            : constructorName.lineEnd
        )({
          typeName: constructorName,
          typeParameters: constructorParams
        })
      );
    }

    const node = buildNode.TypeDeclaration(
      lineStart,
      typeConstructors[typeConstructors.length - 1].lineEnd
    )({
      typeName,
      typeParameters,
      typeConstructors
    });

    collectComments(node);

    return node;
  }

  function parseComment() {
    if (!u.isComment(token)) return;

    const { value, lineStart } = token;

    if (index === 0 || tokens[index - 1].lineStart !== lineStart) {
      fullLineComments[lineStart] = value;
    }

    advance();

    return true;
  }

  function parseExpression() {
    if (!token) return;

    switch (token.type) {
      case tokenTypes.LINE_COMMENT:
        return parseComment();

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

      case tokenTypes.KEYWORD:
        return parseTypeDeclaration() || parseConditional();

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

      const id = buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance(2);

      const valueExpression = parseExpression(token);

      if (!valueExpression) {
        fail('Expected a valid expression on right-hand side of assignment');
      }

      const node = buildNode.Assignment(lineStart, valueExpression.lineEnd)({
        leftSide: id,
        rightSide: valueExpression
      });

      collectComments(node);

      return node;
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
    if (!node) fail(token => `Unexpected ${token} found.`);
    if (node !== true) body.push(node);
  }

  const firstLine = body.length ? body[0].lineStart : 1;
  const lastLine = body.length ? body[body.length - 1].lineEnd : 1;

  return buildNode.Module(firstLine, lastLine)({ body });
}

export default parse;
