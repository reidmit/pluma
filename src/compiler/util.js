import { tokenTypes } from './constants';

export const isTokenType = type => token =>
  token && token.type === tokenTypes[type];

export const isSymbol = value => token =>
  token && token.type === tokenTypes.SYMBOL && token.value === value;

export const isKeyword = value => token =>
  token && token.type === tokenTypes.KEYWORD && token.value === value;

export const isString = isTokenType('STRING');

export const isNumber = isTokenType('NUMBER');

export const isBoolean = isTokenType('BOOLEAN');

export const isIdentifier = isTokenType('IDENTIFIER');

export const isDotIdentifier = isTokenType('DOT_IDENTIFIER');

export const isAtIdentifier = isTokenType('AT_IDENTIFIER');

export const isComment = isTokenType('LINE_COMMENT');

export const isBar = isSymbol('|');

export const isPipe = isSymbol('|>');

export const isLeftBracket = isSymbol('[');

export const isRightBracket = isSymbol(']');

export const isLeftParen = isSymbol('(');

export const isRightParen = isSymbol(')');

export const isLeftBrace = isSymbol('{');

export const isRightBrace = isSymbol('}');

export const isInterpolationStart = isSymbol('${');

export const isEquals = isSymbol('=');

export const isArrow = isSymbol('=>');

export const isThinArrow = isSymbol('->');

export const isComma = isSymbol(',');

export const isColon = isSymbol(':');

export const isDoubleColon = isSymbol('::');

export const isModule = isKeyword('module');

export const isInterop = isKeyword('interop');

export const isExport = isKeyword('export');

export const isImport = isKeyword('import');

export const isFrom = isKeyword('from');

export const isLet = isKeyword('let');

export const isIn = isKeyword('in');

export const isIf = isKeyword('if');

export const isThen = isKeyword('then');

export const isElse = isKeyword('else');

export const isType = isKeyword('type');

export const isAlias = isKeyword('alias');

export const capitalize = string =>
  string[0].toUpperCase() + string.substring(1);
