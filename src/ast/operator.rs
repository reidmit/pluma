use crate::tokens::Token;

pub enum Operator {
  Addition,
  SubtractionOrNegation,
  Multiplication,
  Division,
  Remainder,
  Exponentiation,
  LogicalAnd,
  LogicalOr,
  LogicalNot,
  LessThan,
  GreaterThan,
  LessThanEquals,
  GreaterThanEquals,
  Equality,
  Inequality,
  FunctionCall,
  FieldAccess,
  IndexAccess,
  NullCoalescing,
  Chain,
}

impl Operator {
  pub fn from_token(token: Token) -> Option<Operator> {
    match token {
      Token::Pipe(..) => Some(Operator::Chain),
      Token::Plus(..) => Some(Operator::Addition),
      Token::Minus(..) => Some(Operator::SubtractionOrNegation),
      Token::Star(..) => Some(Operator::Multiplication),
      Token::ForwardSlash(..) => Some(Operator::Division),
      Token::Percent(..) => Some(Operator::Remainder),
      Token::DoubleStar(..) => Some(Operator::Exponentiation),
      Token::DoubleAnd(..) => Some(Operator::LogicalAnd),
      Token::DoublePipe(..) => Some(Operator::LogicalOr),
      Token::LeftAngle(..) => Some(Operator::LessThan),
      Token::RightAngle(..) => Some(Operator::GreaterThan),
      Token::LeftAngleEqual(..) => Some(Operator::LessThanEquals),
      Token::RightAngleEqual(..) => Some(Operator::GreaterThanEquals),
      Token::DoubleEqual(..) => Some(Operator::Equality),
      Token::BangEqual(..) => Some(Operator::Inequality),
      Token::Dot(..) => Some(Operator::FieldAccess),
      Token::LeftBracket(..) => Some(Operator::IndexAccess),
      Token::DoubleQuestion(..) => Some(Operator::NullCoalescing),
      _ => None,
    }
  }

  pub fn infix_binding_power(&self) -> Option<(u8, u8)> {
    use Operator::*;

    // if left < right, it's left-associative
    // if left > right, it's right-associative
    match &self {
      Chain => Some((0, 1)),
      LogicalOr | NullCoalescing => Some((20, 21)),
      LogicalAnd => Some((30, 31)),
      Equality | Inequality => Some((40, 41)),
      LessThan | LessThanEquals | GreaterThan | GreaterThanEquals => Some((50, 51)),
      Addition | SubtractionOrNegation => Some((60, 61)),
      Multiplication | Division | Remainder => Some((70, 71)),
      Exponentiation => Some((81, 80)),
      FieldAccess | IndexAccess => Some((90, 91)),
      // FunctionCall => Some((90, 91)),
      _ => None,
    }
  }

  pub fn prefix_binding_power(&self) -> ((), u8) {
    use Operator::*;

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
      Chain => write!(f, "op-chain"),
      Addition => write!(f, "op-add"),
      SubtractionOrNegation => write!(f, "op-minus"),
      Multiplication => write!(f, "op-multiply"),
      Division => write!(f, "op-divide"),
      Remainder => write!(f, "op-remainder"),
      Exponentiation => write!(f, "op-exponent"),
      LogicalAnd => write!(f, "op-logical-and"),
      LogicalOr => write!(f, "op-logical-or"),
      LogicalNot => write!(f, "op-logical-not"),
      LessThan => write!(f, "op-less-than"),
      GreaterThan => write!(f, "op-greater-than"),
      LessThanEquals => write!(f, "op-less-than-equals"),
      GreaterThanEquals => write!(f, "op-greater-than-equals"),
      Equality => write!(f, "op-equality"),
      Inequality => write!(f, "op-inequality"),
      FunctionCall => write!(f, "op-call"),
      FieldAccess => write!(f, "op-field-access"),
      IndexAccess => write!(f, "op-index-access"),
      NullCoalescing => write!(f, "op-null-coalesce"),
    }
  }
}
