import { tokenTypes } from './constants';

export const isType = type => token => token && token.type === tokenTypes[type];

export const isSymbol = value => token =>
  token && token.type === tokenTypes.SYMBOL && token.value === value;

export const isKeyword = value => token =>
  token && token.type === tokenTypes.KEYWORD && token.value === value;

export const isString = isType('STRING');

export const isNumber = isType('NUMBER');

export const isBoolean = isType('BOOLEAN');

export const isIdentifier = isType('IDENTIFIER');

export const isDotIdentifier = isType('DOT_IDENTIFIER');

export const isAtIdentifier = isType('AT_IDENTIFIER');

export const isComment = isType('LINE_COMMENT');

export const isLeftBrace = isSymbol('[');

export const isRightBrace = isSymbol(']');

export const isLeftParen = isSymbol('(');

export const isRightParen = isSymbol(')');

export const isLeftBracket = isSymbol('{');

export const isRightBracket = isSymbol('}');

export const isInterpolationStart = isSymbol('${');

export const isEquals = isSymbol('=');

export const isArrow = isSymbol('=>');

export const isComma = isSymbol(',');

export const isColon = isSymbol(':');

export const isLet = isKeyword('let');

export const isIf = isKeyword('if');

export const isThen = isKeyword('then');

export const isElse = isKeyword('else');
