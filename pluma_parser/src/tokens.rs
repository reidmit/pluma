use std::fmt;

#[derive(Copy, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum Token {
  And(usize, usize),
  Arrow(usize, usize),
  Bang(usize, usize),
  BinaryDigits(usize, usize),
  Colon(usize, usize),
  Comma(usize, usize),
  Comment(usize, usize),
  DecimalDigits(usize, usize),
  Dot(usize, usize),
  DoubleAnd(usize, usize),
  DoubleArrow(usize, usize),
  DoubleColon(usize, usize),
  DoubleDot(usize, usize),
  DoubleEquals(usize, usize),
  DoubleLeftAngle(usize, usize),
  DoublePipe(usize, usize),
  DoubleRightAngle(usize, usize),
  Equals(usize, usize),
  ForwardSlash(usize, usize),
  HexDigits(usize, usize),
  Identifier(usize, usize),
  IdentifierSpecialOther(usize, usize),
  IdentifierSpecialParam(usize, usize),
  ImportPath(usize, usize),
  InterpolationEnd(usize, usize),
  InterpolationStart(usize, usize),
  KeywordAlias(usize, usize),
  KeywordAs(usize, usize),
  KeywordBreak(usize, usize),
  KeywordConst(usize, usize),
  KeywordDef(usize, usize),
  KeywordEnum(usize, usize),
  KeywordInternal(usize, usize),
  KeywordIntrinsicDef(usize, usize),
  KeywordIntrinsicType(usize, usize),
  KeywordLet(usize, usize),
  KeywordMatch(usize, usize),
  KeywordMut(usize, usize),
  KeywordPrivate(usize, usize),
  KeywordStruct(usize, usize),
  KeywordTrait(usize, usize),
  KeywordUse(usize, usize),
  KeywordWhere(usize, usize),
  LeftAngle(usize, usize),
  LeftAngleEqual(usize, usize),
  LeftBrace(usize, usize),
  LeftBracket(usize, usize),
  LeftParen(usize, usize),
  LineBreak(usize, usize),
  OctalDigits(usize, usize),
  Percent(usize, usize),
  Pipe(usize, usize),
  RightAngle(usize, usize),
  RightAngleEqual(usize, usize),
  RightBrace(usize, usize),
  RightBracket(usize, usize),
  RightParen(usize, usize),
  Star(usize, usize),
  StringLiteral(usize, usize),
  Underscore(usize, usize),
  Unexpected(usize, usize),
}

impl Token {
  pub fn get_position(&self) -> (usize, usize) {
    use Token::*;

    match self {
      And(start, end)
      | Arrow(start, end)
      | Bang(start, end)
      | BinaryDigits(start, end)
      | Colon(start, end)
      | Comma(start, end)
      | Comment(start, end)
      | DecimalDigits(start, end)
      | Dot(start, end)
      | DoubleAnd(start, end)
      | DoubleArrow(start, end)
      | DoubleColon(start, end)
      | DoubleDot(start, end)
      | DoubleEquals(start, end)
      | DoublePipe(start, end)
      | Equals(start, end)
      | ForwardSlash(start, end)
      | HexDigits(start, end)
      | Identifier(start, end)
      | IdentifierSpecialOther(start, end)
      | IdentifierSpecialParam(start, end)
      | ImportPath(start, end)
      | InterpolationEnd(start, end)
      | InterpolationStart(start, end)
      | KeywordAlias(start, end)
      | KeywordAs(start, end)
      | KeywordBreak(start, end)
      | KeywordConst(start, end)
      | KeywordDef(start, end)
      | KeywordEnum(start, end)
      | KeywordInternal(start, end)
      | KeywordIntrinsicDef(start, end)
      | KeywordIntrinsicType(start, end)
      | KeywordLet(start, end)
      | KeywordMatch(start, end)
      | KeywordMut(start, end)
      | KeywordPrivate(start, end)
      | KeywordStruct(start, end)
      | KeywordTrait(start, end)
      | KeywordUse(start, end)
      | KeywordWhere(start, end)
      | LeftAngle(start, end)
      | LeftBrace(start, end)
      | LeftBracket(start, end)
      | LeftParen(start, end)
      | LineBreak(start, end)
      | OctalDigits(start, end)
      | Percent(start, end)
      | Pipe(start, end)
      | RightAngle(start, end)
      | RightBrace(start, end)
      | RightBracket(start, end)
      | RightParen(start, end)
      | Star(start, end)
      | StringLiteral(start, end)
      | Underscore(start, end)
      | Unexpected(start, end) => (*start, *end),
    }
  }
}

impl fmt::Display for Token {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    use Token::*;

    let as_string = match self {
      &And(..) => "a '&'",
      &Arrow(..) => "a '->'",
      &Bang(..) => "a '!'",
      &BinaryDigits(..) => "binary digits",
      &Colon(..) => "a ':'",
      &Comma(..) => "a ','",
      &Comment(..) => "a comment",
      &DecimalDigits(..) => "digits",
      &Dot(..) => "a '.'",
      &DoubleAnd(..) => "a '&&'",
      &DoubleArrow(..) => "a '=>'",
      &DoubleColon(..) => "a '::'",
      &DoubleDot(..) => "a '..'",
      &DoubleEquals(..) => "a '=='",
      &DoublePipe(..) => "a '||'",
      &Equals(..) => "a '='",
      &ForwardSlash(..) => "a '/'",
      &HexDigits(..) => "hex digits",
      &Identifier(..) => "an identifier",
      &IdentifierSpecialOther(..) => "an identifier starting with '$'",
      &IdentifierSpecialParam(..) => "an identifier starting with '$'",
      &ImportPath(..) => "an import path",
      &InterpolationEnd(..) => "a ')'",
      &InterpolationStart(..) => "a '$('",
      &KeywordAlias(..) => "keyword 'alias'",
      &KeywordAs(..) => "keyword 'as'",
      &KeywordBreak(..) => "keyword 'break'",
      &KeywordConst(..) => "keyword 'const'",
      &KeywordDef(..) => "keyword 'def'",
      &KeywordEnum(..) => "keyword 'enum'",
      &KeywordInternal(..) => "keyword 'internal'",
      &KeywordIntrinsicDef(..) => "keyword 'intrinsic_def'",
      &KeywordIntrinsicType(..) => "keyword 'intrinsic_type'",
      &KeywordLet(..) => "keyword 'let'",
      &KeywordMatch(..) => "keyword 'match'",
      &KeywordMut(..) => "keyword 'mut'",
      &KeywordPrivate(..) => "keyword 'private'",
      &KeywordStruct(..) => "keyword 'struct'",
      &KeywordTrait(..) => "keyword 'trait'",
      &KeywordUse(..) => "keyword 'use'",
      &KeywordWhere(..) => "keyword 'where'",
      &LeftAngle(..) => "a '<'",
      &LeftBrace(..) => "a '{'",
      &LeftBracket(..) => "a '['",
      &LeftParen(..) => "a '('",
      &LineBreak(..) => "a line break",
      &OctalDigits(..) => "octal digits",
      &Pipe(..) => "a '|'",
      &RightAngle(..) => "a '>'",
      &RightBrace(..) => "a '}'",
      &RightBracket(..) => "a ']'",
      &RightParen(..) => "a ')'",
      &StringLiteral(..) => "a string",
      &Underscore(..) => "a '_'",
      &Unexpected(..) => "unknown",
    };

    write!(f, "{}", as_string)
  }
}
