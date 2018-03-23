import { tokenTypes } from './constants';

export const isType = type => token => token && token.type === tokenTypes[type];

export const isSymbol = value => token =>
  token && token.type === tokenTypes.SYMBOL && token.value === value;

export const isKeyword = value => token =>
  token && token.type === tokenTypes.KEYWORD && token.value === value;

export const isString = isType('STRING');

export const isNumber = isType('NUMBER');

export const isIdentifier = isType('IDENTIFIER');

export const isDot = isSymbol('.');

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
