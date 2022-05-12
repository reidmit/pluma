use crate::tokens::Token;

pub enum Operator {
  Addition,
  Chain,
  Division,
  Equality,
  Exponentiation,
  FieldAccess,
  FunctionCall,
  GreaterThan,
  GreaterThanEquals,
  IndexAccess,
  Inequality,
  LessThan,
  LessThanEquals,
  LogicalAnd,
  LogicalNot,
  LogicalOr,
  Multiplication,
  NullCoalescing,
  Range,
  Remainder,
  SubtractionOrNegation,
}

impl Operator {
  pub fn from_token(token: Token) -> Option<Operator> {
    match token {
      Token::BangEqual(..) => Some(Operator::Inequality),
      Token::Dot(..) => Some(Operator::FieldAccess),
      Token::DoubleAnd(..) => Some(Operator::LogicalAnd),
      Token::DoubleDot(..) => Some(Operator::Range),
      Token::DoubleEqual(..) => Some(Operator::Equality),
      Token::DoublePipe(..) => Some(Operator::LogicalOr),
      Token::DoubleQuestion(..) => Some(Operator::NullCoalescing),
      Token::DoubleStar(..) => Some(Operator::Exponentiation),
      Token::ForwardSlash(..) => Some(Operator::Division),
      Token::LeftAngle(..) => Some(Operator::LessThan),
      Token::LeftAngleEqual(..) => Some(Operator::LessThanEquals),
      Token::LeftBracket(..) => Some(Operator::IndexAccess),
      Token::Minus(..) => Some(Operator::SubtractionOrNegation),
      Token::Percent(..) => Some(Operator::Remainder),
      Token::Pipe(..) => Some(Operator::Chain),
      Token::Plus(..) => Some(Operator::Addition),
      Token::RightAngle(..) => Some(Operator::GreaterThan),
      Token::RightAngleEqual(..) => Some(Operator::GreaterThanEquals),
      Token::Star(..) => Some(Operator::Multiplication),
      _ => None,
    }
  }

  pub fn infix_binding_power(&self) -> Option<(u8, u8)> {
    use Operator::*;

    // if left < right, it's left-associative
    // if left > right, it's right-associative
    // lower numbers bind weaker than higher numbers
    match &self {
      Chain => Some((0, 1)),
      Range => Some((10, 11)),
      LogicalOr | NullCoalescing => Some((20, 21)),
      LogicalAnd => Some((30, 31)),
      Equality | Inequality => Some((40, 41)),
      LessThan | LessThanEquals | GreaterThan | GreaterThanEquals => Some((50, 51)),
      Addition | SubtractionOrNegation => Some((60, 61)),
      Multiplication | Division | Remainder => Some((70, 71)),
      Exponentiation => Some((81, 80)),
      FunctionCall | FieldAccess | IndexAccess => Some((90, 91)),
      _ => None,
    }
  }

  pub fn prefix_binding_power(&self) -> ((), u8) {
    use Operator::*;

    // these numbers are relative to those above (see infix_binding_power);
    match &self {
      SubtractionOrNegation => ((), 75),
      LogicalNot => ((), 35),
      _ => unreachable!(),
    }
  }
}

impl std::fmt::Debug for Operator {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use Operator::*;

    match &self {
      Addition => write!(f, "op-add"),
      Chain => write!(f, "op-chain"),
      Division => write!(f, "op-divide"),
      Equality => write!(f, "op-equality"),
      Exponentiation => write!(f, "op-exponent"),
      FieldAccess => write!(f, "op-field-access"),
      FunctionCall => write!(f, "op-call"),
      GreaterThan => write!(f, "op-greater-than"),
      GreaterThanEquals => write!(f, "op-greater-than-equals"),
      IndexAccess => write!(f, "op-index-access"),
      Inequality => write!(f, "op-inequality"),
      LessThan => write!(f, "op-less-than"),
      LessThanEquals => write!(f, "op-less-than-equals"),
      LogicalAnd => write!(f, "op-logical-and"),
      LogicalNot => write!(f, "op-logical-not"),
      LogicalOr => write!(f, "op-logical-or"),
      Multiplication => write!(f, "op-multiply"),
      NullCoalescing => write!(f, "op-null-coalesce"),
      Range => write!(f, "op-range"),
      Remainder => write!(f, "op-remainder"),
      SubtractionOrNegation => write!(f, "op-minus"),
    }
  }
}
