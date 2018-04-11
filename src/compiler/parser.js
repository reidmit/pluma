import * as u from './util';
import { buildNode } from './ast-nodes';
import ParserError from './parser-error';
import { tokenToString } from './error-helper';
import { tokenTypes } from './constants';

function parse({ source, tokens }) {
  let index = 0;
  let token = tokens[index];
  let lastAssignmentColumn = 0;
  const fullLineComments = {};
  let interop = false;
  let moduleName = null;
  const exports = [];
  const imports = [];

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

  function parsePossibleCallExpression(disallowCalls) {
    const firstTokenColumnStart = token.columnStart;

    let func =
      parseFunction() || parseGetter() || parseSetter() || parseIdentifier();

    if (!func) return;

    if (disallowCalls) return func;

    let argument;
    while (
      token &&
      token.columnStart > lastAssignmentColumn &&
      token.columnStart > firstTokenColumnStart
    ) {
      const lineStart = token.lineStart;
      const lineEnd = token.lineEnd;

      argument = parseExpression({ disallowPipes: true, disallowCalls: true });

      if (!argument || argument === true) break;

      func = buildNode.Call(lineStart, lineEnd)({ callee: func, argument });
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

      return buildNode.RecordProperty(key.lineStart, value.lineEnd)({
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

      return buildNode.RecordProperty(key.lineStart, key.lineEnd)({
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
        fail('Failed to parse record property.');
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

    return buildNode.Record(lineStart, lineEnd)({ properties });
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

        firstNode = buildNode.TupleType(leftParen.lineStart, leftParen.lineEnd)(
          {
            typeEntries: [firstNode, ...otherTupleEntries]
          }
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

      const properties = [];

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

        properties.push(
          buildNode.RecordPropertyType(name.lineStart, value.lineEnd)({
            key: name,
            value
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
        properties[properties.length - 1].lineEnd
      )({
        properties
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

      return buildNode.FunctionType(firstNode.lineStart, rightSide.lineEnd)({
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

  function parseLetExpression() {
    if (!u.isLet(token)) return;

    const lineStart = token.lineStart;

    advance();

    const assignments = [];

    let assignment;
    while ((assignment = parseAssignment())) {
      assignments.push(assignment);
    }

    if (!assignments.length) {
      fail('Expected at least one assignment after "let" keyword.');
    }

    if (!u.isIn(token)) {
      fail('Expected "in" keyword after assignments in "let" expression.');
    }

    advance();

    const body = parseExpression();

    if (!body) {
      fail('Expected a valid expression after "in" in "let" expression.');
    }

    return buildNode.LetExpression(lineStart, body.lineEnd)({
      assignments,
      body
    });
  }

  function parseExpression(options = {}) {
    if (!token) return;

    let expr = null;

    switch (token.type) {
      case tokenTypes.LINE_COMMENT:
        return parseComment();

      case tokenTypes.NUMBER:
        expr = parseNumber();
        break;

      case tokenTypes.BOOLEAN:
        expr = parseBoolean();
        break;

      case tokenTypes.STRING:
        expr = parseString();
        break;

      case tokenTypes.DOT_IDENTIFIER:
      case tokenTypes.AT_IDENTIFIER:
      case tokenTypes.IDENTIFIER:
        expr = parsePossibleCallExpression(options.disallowCalls);
        break;

      case tokenTypes.KEYWORD:
        expr =
          parseTypeDeclaration() || parseConditional() || parseLetExpression();
        break;

      case tokenTypes.SYMBOL:
        expr = parseParenthetical() || parseObject() || parseArray();
        break;
    }

    if (!options.disallowPipes && expr && u.isPipe(token)) {
      advance();

      const rightSide = parseExpression({ disallowPipes: true });

      if (!rightSide) {
        fail('Expected a valid expression after "|>".');
      }

      return buildNode.PipeExpression(expr.lineStart, rightSide.lineEnd)({
        left: expr,
        right: rightSide
      });
    }

    return expr;
  }

  function parseAssignment() {
    if (
      u.isIdentifier(token) &&
      (u.isEquals(tokens[index + 1]) || u.isDoubleColon(tokens[index + 1]))
    ) {
      const lineStart = token.lineStart;

      lastAssignmentColumn = token.columnStart;

      const id = buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      });

      advance();

      let typeAnnotation = null;

      if (u.isDoubleColon(token)) {
        advance();

        typeAnnotation = parseTypeExpression();

        if (!typeAnnotation) {
          fail('Expected a valid type annotation after "::" in assignment.');
        }
      }

      advance();

      const valueExpression = parseExpression();

      if (!valueExpression) {
        fail('Expected a valid expression on right-hand side of assignment');
      }

      const node = buildNode.Assignment(lineStart, valueExpression.lineEnd)({
        id,
        typeAnnotation,
        value: valueExpression
      });

      collectComments(node);

      return node;
    }
  }

  function parseModuleDeclaration() {
    if (
      !(
        u.isModule(token) ||
        (u.isInterop(token) && u.isModule(tokens[index + 1]))
      )
    )
      return;

    if (moduleName) {
      fail(
        'Duplicate "module" declaration. A file may only contain one "module" declaration.'
      );
    }

    if (body.length) {
      fail(
        'Invalid "module" declaration. A "module" declaration must be the first statement in its file.'
      );
    }

    if (u.isInterop(token)) {
      interop = true;
      advance();
    }

    advance();

    if (!u.isIdentifier(token)) {
      fail('Expected a valid identifier after "module" in module declaration.');
    }

    moduleName = buildNode.Identifier(token.lineStart, token.lineEnd)({
      value: token.value,
      isGetter: false,
      isSetter: false
    });

    advance();

    return true;
  }

  function parseExport() {
    if (!u.isExport(token)) return;

    if (imports.length) {
      fail(
        'Export statements must come before any import statements in a file.'
      );
    }

    advance();

    if (!u.isLeftParen(token)) {
      fail('Expected "(" after "export" in export statement.');
    }

    advance();

    if (!u.isIdentifier(token)) {
      fail('Expected an identifier after "(" in export statement.');
    }

    while (u.isIdentifier(token)) {
      exports.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();

      if (u.isRightParen(token)) break;

      if (u.isComma(token)) advance();
    }

    if (!u.isRightParen(token)) {
      fail('Expected closing ")" to match opening "(" in export statement.');
    }

    advance();

    return true;
  }

  function parseImport() {
    if (!u.isImport(token)) return;

    if (body.length) {
      fail(
        'Import statements must appear after export statements and before any other content in a file.'
      );
    }

    const lineStart = token.lineStart;

    advance();

    if (u.isIdentifier(token)) {
      imports.push(
        buildNode.Import(lineStart, token.lineEnd)({
          identifiers: null,
          module: buildNode.Identifier(token.lineStart, token.lineEnd)({
            value: token.value,
            isGetter: false,
            isSetter: false
          })
        })
      );

      advance();

      return true;
    }

    if (!u.isLeftParen(token)) {
      fail('Expected a module name or "(" after "import" in import statement.');
    }

    advance();

    if (!u.isIdentifier(token)) {
      fail('Expected an identifier name after "(" in import statement.');
    }

    const identifiers = [];

    while (u.isIdentifier(token)) {
      identifiers.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();

      if (u.isRightParen(token)) break;

      if (u.isComma(token)) advance();
    }

    if (!u.isRightParen(token)) {
      fail('Expected closing ")" to match opening "(" in import statement.');
    }

    advance();

    if (!u.isFrom(token)) {
      fail('Expected keyword "from" after ")" in import statement.');
    }

    advance();

    if (!u.isIdentifier(token)) {
      fail('Expected identifier after "from" in import statement.');
    }

    const moduleNameParts = [];

    moduleNameParts.push(
      buildNode.Identifier(token.lineStart, token.lineEnd)({
        value: token.value,
        isGetter: false,
        isSetter: false
      })
    );

    advance();

    while (token && u.isDotIdentifier(token)) {
      moduleNameParts.push(
        buildNode.Identifier(token.lineStart, token.lineEnd)({
          value: token.value,
          isGetter: false,
          isSetter: false
        })
      );

      advance();

      if (!token) break;
    }

    let moduleNode;
    if (moduleNameParts.length === 1) {
      moduleNode = moduleNameParts[0];
    } else {
      moduleNode = buildNode.MemberExpression(
        moduleNameParts[0].lineStart,
        moduleNameParts[moduleNameParts.length - 1].lineEnd
      )({
        parts: moduleNameParts
      });
    }

    imports.push(
      buildNode.Import(lineStart, moduleNode.lineEnd)({
        identifiers,
        module: moduleNode
      })
    );

    return true;
  }

  function parseStatement() {
    const moduleDecl = parseModuleDeclaration();
    if (moduleDecl) return true;

    const exportStatement = parseExport();
    if (exportStatement) return true;

    const importStatement = parseImport();
    if (importStatement) return true;

    const assignment = parseAssignment();
    if (assignment) return assignment;

    const expression = parseExpression();
    if (expression) return expression;
  }

  const body = [];
  while (index < tokens.length) {
    const node = parseStatement(token);
    if (!node) fail(token => `Unexpected ${token} found.`);
    if (node !== true) body.push(node);
  }

  const firstLine = moduleName
    ? moduleName.lineStart
    : exports.length
      ? exports[0].lineStart
      : imports.length
        ? imports[0].lineStart
        : body.length ? body[0].lineStart : 1;

  const lastLine = body.length
    ? body[body.length - 1].lineEnd
    : imports.length
      ? imports[imports.length - 1].lineEnd
      : Math.max(firstLine, 1);

  const moduleNode = buildNode.Module(firstLine, lastLine)({
    interop,
    name: moduleName,
    exports,
    imports,
    body
  });

  collectComments(moduleNode);

  return moduleNode;
}

export default parse;
